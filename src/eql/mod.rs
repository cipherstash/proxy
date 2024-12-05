use cipherstash_client::zerokms::{encrypted_record, EncryptedRecord};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plaintext {
    #[serde(rename = "p")]
    pub plaintext: String,
    #[serde(rename = "i")]
    pub identifier: Identifier,
    #[serde(rename = "v")]
    pub version: u16,
    #[serde(rename = "q")]
    pub for_query: Option<ForQuery>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Identifier {
    #[serde(rename = "t")]
    pub table: String,
    #[serde(rename = "c")]
    pub column: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForQuery {
    Match,
    Ore,
    Unique,
    SteVec, // Should this be SteVecContainment?
    EjsonPath,
    SteVecTerm,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename = "ct")]
pub struct Ciphertext {
    #[serde(rename = "c", with = "encrypted_record::formats::mp_base85")]
    pub ciphertext: EncryptedRecord,
    #[serde(rename = "k", default = "Ciphertext::default_kind")]
    pub kind: String,
    #[serde(rename = "o")]
    pub ore_index: Option<String>,
    #[serde(rename = "m")]
    pub match_index: Option<Vec<u16>>,
    #[serde(rename = "u")]
    pub unique_index: Option<String>,
    #[serde(rename = "i")]
    pub identifier: Identifier,
    #[serde(rename = "v")]
    pub version: u16,
}

impl Ciphertext {
    pub fn new(ciphertext: EncryptedRecord, identifier: Identifier) -> Self {
        Self {
            ciphertext,
            kind: Self::default_kind(),
            identifier,
            version: 1,
            ore_index: None,
            match_index: None,
            unique_index: None,
        }
    }

    pub fn default_kind() -> String {
        "ct".to_string()
    }
}
