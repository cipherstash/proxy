use cipherstash_client::zerokms::{encrypted_record, EncryptedRecord};
use serde::{Deserialize, Serialize};
use sqltk::parser::ast::Ident;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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

impl Identifier {
    pub fn new<S>(table: S, column: S) -> Self
    where
        S: Into<String>,
    {
        let table = table.into();
        let column = column.into();

        Self { table, column }
    }

    pub fn table(&self) -> &String {
        &self.table
    }

    pub fn column(&self) -> &String {
        &self.column
    }
}

impl From<(&Ident, &Ident)> for Identifier {
    fn from((table, column): (&Ident, &Ident)) -> Self {
        Self {
            table: table.value.to_owned(),
            column: column.value.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForQuery {
    Match,
    Ore,
    Unique,
    SteVec, // Should this be SteVecContainment?
    EjsonPath,
    SteVecTerm,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EqlEncrypted {
    #[serde(rename = "i")]
    pub(crate) identifier: Identifier,
    #[serde(rename = "v")]
    pub(crate) version: u16,

    #[serde(flatten)]
    pub(crate) body: EqlEncryptedBody,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EqlEncryptedBody {
    #[serde(rename = "c", with = "encrypted_record::formats::mp_base85")]
    pub(crate) ciphertext: EncryptedRecord,

    #[serde(flatten)]
    pub(crate) indexes: EqlEncryptedIndexes,

    #[serde(rename = "a", skip_serializing_if = "Option::is_none")]
    pub(crate) is_array_item: Option<bool>,
}

///
/// EqlEncryptedIndexes
///   - null values should not be serialized
///   - the null carries through to the database as this is the EQL JSON format
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct EqlEncryptedIndexes {
    #[serde(rename = "ob", skip_serializing_if = "Option::is_none")]
    pub(crate) ore_block_u64_8_256: Option<Vec<String>>,

    #[serde(rename = "bf", skip_serializing_if = "Option::is_none")]
    pub(crate) bloom_filter: Option<Vec<u16>>,

    #[serde(rename = "hm", skip_serializing_if = "Option::is_none")]
    pub(crate) hmac_256: Option<String>,

    #[serde(rename = "s", skip_serializing_if = "Option::is_none")]
    pub(crate) selector: Option<String>,

    #[serde(rename = "b3", skip_serializing_if = "Option::is_none")]
    pub(crate) blake3: Option<String>,

    #[serde(rename = "ocf", skip_serializing_if = "Option::is_none")]
    pub(crate) ore_cllw_u64_8: Option<String>,

    #[serde(rename = "ocv", skip_serializing_if = "Option::is_none")]
    pub(crate) ore_cllw_var_8: Option<String>,

    #[serde(rename = "sv", skip_serializing_if = "Option::is_none")]
    pub(crate) ste_vec_index: Option<Vec<EqlEncryptedBody>>,
}

#[cfg(test)]
mod tests {
    use crate::{
        eql::{EqlEncryptedBody, EqlEncryptedIndexes},
        EqlEncrypted,
    };

    use super::{Identifier, Plaintext};
    use cipherstash_client::zerokms::EncryptedRecord;
    use recipher::key::Iv;
    use uuid::Uuid;

    #[test]
    pub fn plaintext_json() {
        let identifier = Identifier::new("table", "column");
        let pt = Plaintext {
            identifier,
            plaintext: "plaintext".to_string(),
            version: 1,
            for_query: None,
        };

        let value = serde_json::to_value(&pt).unwrap();

        let i = &value["i"];
        let t = &i["t"];
        assert_eq!(t, "table");

        let result: Plaintext = serde_json::from_value(value).unwrap();
        assert_eq!(pt, result);
    }

    #[test]
    pub fn ciphertext_json() {
        let expected = Identifier::new("table", "column");

        let ct = EqlEncrypted {
            identifier: expected.clone(),
            version: 1,
            body: EqlEncryptedBody {
                ciphertext: EncryptedRecord {
                    iv: Iv::default(),
                    ciphertext: vec![1; 32],
                    tag: vec![1; 16],
                    descriptor: "ciphertext".to_string(),
                    dataset_id: Some(Uuid::new_v4()),
                },
                indexes: EqlEncryptedIndexes {
                    ore_block_u64_8_256: None,
                    bloom_filter: None,
                    hmac_256: None,
                    blake3: None,
                    selector: None,
                    ore_cllw_u64_8: None,
                    ore_cllw_var_8: None,
                    ste_vec_index: None,
                },
                is_array_item: None,
            },
        };

        let value = serde_json::to_value(&ct).unwrap();

        let i = &value["i"];
        let t = &i["t"];
        assert_eq!(t, "table");

        let result: EqlEncrypted = serde_json::from_value(value).unwrap();
        assert_eq!(expected, result.identifier);
    }
}
