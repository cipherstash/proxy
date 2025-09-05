#[allow(clippy::module_inception)]
mod zerokms;

pub use zerokms::ZeroKms;

use crate::{
    config::TandemConfig,
    eql::{self, EqlEncryptedBody, EqlEncryptedIndexes},
    error::{EncryptError, Error},
    Identifier, EQL_SCHEMA_VERSION,
};
use cipherstash_client::config::{ConfigError, ZeroKMSConfigWithClientKey};
use cipherstash_client::{
    config::EnvSource,
    credentials::{auto_refresh::AutoRefresh, ServiceCredentials},
    encryption::{Encrypted, EncryptedEntry, EncryptedSteVecTerm, IndexTerm, Plaintext},
    zerokms::ClientKey,
    ConsoleConfig, CtsConfig, ZeroKMS, ZeroKMSConfig,
};

pub type ScopedCipher =
    cipherstash_client::encryption::ScopedCipher<AutoRefresh<ServiceCredentials>>;

pub type ZerokmsClient = ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>;

pub(crate) fn init_zerokms_client(
    config: &TandemConfig,
) -> Result<ZeroKMS<AutoRefresh<ServiceCredentials>, ClientKey>, ConfigError> {
    let zerokms_config = build_zerokms_config(config)?;

    Ok(zerokms_config
        .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials())))
}

pub fn build_zerokms_config(
    config: &TandemConfig,
) -> Result<ZeroKMSConfigWithClientKey, ConfigError> {
    let console_config = ConsoleConfig::builder().with_env().build()?;

    let builder = CtsConfig::builder().with_env();
    let builder = if let Some(cts_host) = config.cts_host() {
        builder.base_url(&cts_host)
    } else {
        builder
    };
    let cts_config = builder.build()?;

    // Not using with_env because the proxy config should take precedence
    let builder = ZeroKMSConfig::builder()
        .add_source(EnvSource::default())
        .workspace_crn(config.auth.workspace_crn.clone())
        .access_key(&config.auth.client_access_key)
        .try_with_client_id(&config.encrypt.client_id)?
        .try_with_client_key(&config.encrypt.client_key)?
        .console_config(&console_config)
        .cts_config(&cts_config);

    let builder = if let Some(zerokms_host) = config.zerokms_host() {
        builder.base_url(zerokms_host)
    } else {
        builder
    };

    builder.build_with_client_key()
}

pub(crate) fn to_eql_encrypted_from_index_term(
    index_term: IndexTerm,
    identifier: &Identifier,
) -> Result<eql::EqlEncrypted, Error> {
    let selector = match index_term {
        IndexTerm::SteVecSelector(s) => Some(hex::encode(s.as_bytes())),
        _ => return Err(EncryptError::InvalidIndexTerm.into()),
    };

    Ok(eql::EqlEncrypted {
        identifier: identifier.to_owned(),
        version: EQL_SCHEMA_VERSION,
        body: EqlEncryptedBody {
            ciphertext: None,
            indexes: EqlEncryptedIndexes {
                bloom_filter: None,
                ore_block_u64_8_256: None,
                hmac_256: None,
                blake3: None,
                ore_cllw_u64_8: None,
                ore_cllw_var_8: None,
                selector,
                ste_vec_index: None,
            },
            is_array_item: None,
        },
    })
}

pub(crate) fn to_eql_encrypted(
    encrypted: Encrypted,
    identifier: &Identifier,
) -> Result<eql::EqlEncrypted, Error> {
    match encrypted {
        Encrypted::Record(ciphertext, terms) => {
            let mut match_index: Option<Vec<u16>> = None;
            let mut ore_index: Option<Vec<String>> = None;
            let mut unique_index: Option<String> = None;
            let mut blake3_index: Option<String> = None;
            let mut ore_cclw_fixed_index: Option<String> = None;
            let mut ore_cclw_var_index: Option<String> = None;
            let mut selector: Option<String> = None;

            for index_term in terms {
                match index_term {
                    IndexTerm::Binary(bytes) => {
                        unique_index = Some(format_index_term_binary(&bytes))
                    }
                    IndexTerm::BitMap(inner) => match_index = Some(inner),
                    IndexTerm::OreArray(bytes) => {
                        ore_index = Some(format_index_term_ore_array(&bytes));
                    }
                    IndexTerm::OreFull(bytes) => {
                        ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::OreLeft(bytes) => {
                        ore_index = Some(format_index_term_ore(&bytes));
                    }
                    IndexTerm::BinaryVec(_) => todo!(),
                    IndexTerm::SteVecSelector(s) => {
                        selector = Some(hex::encode(s.as_bytes()));
                    }
                    IndexTerm::SteVecTerm(ste_vec_term) => match ste_vec_term {
                        EncryptedSteVecTerm::Mac(bytes) => blake3_index = Some(hex::encode(bytes)),
                        EncryptedSteVecTerm::OreFixed(ore) => {
                            ore_cclw_fixed_index = Some(hex::encode(&ore))
                        }
                        EncryptedSteVecTerm::OreVariable(ore) => {
                            ore_cclw_var_index = Some(hex::encode(&ore))
                        }
                    },
                    IndexTerm::SteQueryVec(_query) => {} // TODO: what do we do here?
                    IndexTerm::Null => {}
                };
            }

            Ok(eql::EqlEncrypted {
                identifier: identifier.to_owned(),
                version: EQL_SCHEMA_VERSION,
                body: EqlEncryptedBody {
                    ciphertext: Some(ciphertext),
                    indexes: EqlEncryptedIndexes {
                        bloom_filter: match_index,
                        ore_block_u64_8_256: ore_index,
                        hmac_256: unique_index,
                        blake3: blake3_index,
                        ore_cllw_u64_8: ore_cclw_fixed_index,
                        ore_cllw_var_8: ore_cclw_var_index,
                        selector,
                        ste_vec_index: None,
                    },
                    is_array_item: None,
                },
            })
        }
        Encrypted::SteVec(ste_vec) => {
            let ciphertext = ste_vec.root_ciphertext()?.clone();

            let ste_vec_index: Vec<EqlEncryptedBody> = ste_vec
                .into_iter()
                .map(
                    |EncryptedEntry {
                         tokenized_selector,
                         term,
                         record,
                         parent_is_array,
                     }| {
                        let indexes = match term {
                            EncryptedSteVecTerm::Mac(bytes) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                blake3: Some(hex::encode(bytes)),
                                ..Default::default()
                            },
                            EncryptedSteVecTerm::OreFixed(ore) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                ore_cllw_u64_8: Some(hex::encode(&ore)),
                                ..Default::default()
                            },
                            EncryptedSteVecTerm::OreVariable(ore) => EqlEncryptedIndexes {
                                selector: Some(hex::encode(tokenized_selector.as_bytes())),
                                ore_cllw_var_8: Some(hex::encode(&ore)),
                                ..Default::default()
                            },
                        };

                        eql::EqlEncryptedBody {
                            ciphertext: Some(record),
                            indexes,
                            is_array_item: Some(parent_is_array),
                        }
                    },
                )
                .collect();

            // FIXME: I'm unsure if I've handled the root ciphertext correctly
            // The way it's implemented right now is that it will be repeated one in the ste_vec_index.
            Ok(eql::EqlEncrypted {
                identifier: identifier.to_owned(),
                version: EQL_SCHEMA_VERSION,
                body: EqlEncryptedBody {
                    ciphertext: Some(ciphertext.clone()),
                    indexes: EqlEncryptedIndexes {
                        bloom_filter: None,
                        ore_block_u64_8_256: None,
                        hmac_256: None,
                        blake3: None,
                        ore_cllw_u64_8: None,
                        ore_cllw_var_8: None,
                        selector: None,
                        ste_vec_index: Some(ste_vec_index),
                    },
                    is_array_item: None,
                },
            })
        }
    }
}

fn format_index_term_binary(bytes: &Vec<u8>) -> String {
    hex::encode(bytes)
}

fn format_index_term_ore_bytea(bytes: &Vec<u8>) -> String {
    hex::encode(bytes)
}

///
/// Formats a Vec<Vec<u8>> into a Vec<String>
///
fn format_index_term_ore_array(vec_of_bytes: &[Vec<u8>]) -> Vec<String> {
    vec_of_bytes
        .iter()
        .map(format_index_term_ore_bytea)
        .collect()
}

///
/// Formats a Vec<Vec<u8>> into a single elenent Vec<String>
///
fn format_index_term_ore(bytes: &Vec<u8>) -> Vec<String> {
    vec![format_index_term_ore_bytea(bytes)]
}

pub(crate) fn plaintext_type_name(pt: Plaintext) -> String {
    match pt {
        Plaintext::BigInt(_) => "BigInt".to_string(),
        Plaintext::BigUInt(_) => "BigUInt".to_string(),
        Plaintext::Boolean(_) => "Boolean".to_string(),
        Plaintext::Decimal(_) => "Decimal".to_string(),
        Plaintext::Float(_) => "Float".to_string(),
        Plaintext::Int(_) => "Int".to_string(),
        Plaintext::NaiveDate(_) => "NaiveDate".to_string(),
        Plaintext::SmallInt(_) => "SmallInt".to_string(),
        Plaintext::Timestamp(_) => "Timestamp".to_string(),
        Plaintext::Utf8Str(_) => "Utf8Str".to_string(),
        Plaintext::JsonB(_) => "JsonB".to_string(),
    }
}
