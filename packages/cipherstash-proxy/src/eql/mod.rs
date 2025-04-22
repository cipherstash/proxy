use cipherstash_client::{
    encryption::SteVec,
    zerokms::{encrypted_record, EncryptedRecord},
};
use serde::{Deserialize, Serialize};
use sqltk_parser::ast::Ident;

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
#[serde(tag = "k")]
pub enum Encrypted {
    #[serde(rename = "ct")]
    Ciphertext {
        #[serde(rename = "c", with = "encrypted_record::formats::mp_base85")]
        ciphertext: EncryptedRecord,
        #[serde(rename = "o")]
        ore_index: Option<Vec<String>>,
        #[serde(rename = "m")]
        match_index: Option<Vec<u16>>,
        #[serde(rename = "u")]
        unique_index: Option<String>,
        #[serde(rename = "i")]
        identifier: Identifier,
        #[serde(rename = "v")]
        version: u16,
    },
    #[serde(rename = "sv")]
    SteVec {
        #[serde(rename = "sv")]
        ste_vec_index: SteVec<16>,
        #[serde(rename = "i")]
        identifier: Identifier,
        #[serde(rename = "v")]
        version: u16,
    },
}

// fn ident_de<'de, D>(deserializer: D) -> Result<Ident, D::Error>
// where
//     D: serde::Deserializer<'de>,
// {
//     let s = String::deserialize(deserializer)?;
//     Ok(Ident::with_quote('"', s))
// }

// fn ident_se<S>(ident: &Ident, serializer: S) -> Result<S::Ok, S::Error>
// where
//     S: Serializer,
// {
//     let s = ident.to_string();
//     serializer.serialize_str(&s)
// }

#[cfg(test)]
mod tests {
    use super::{Identifier, Plaintext};
    use crate::Encrypted;
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

        let ct = Encrypted::Ciphertext {
            identifier: expected.clone(),
            version: 1,
            ciphertext: EncryptedRecord {
                iv: Iv::default(),
                ciphertext: vec![1; 32],
                tag: vec![1; 16],
                descriptor: "ciphertext".to_string(),
                dataset_id: Some(Uuid::new_v4()),
            },

            ore_index: None,
            match_index: None,
            unique_index: None,
        };

        let value = serde_json::to_value(&ct).unwrap();

        let i = &value["i"];
        let t = &i["t"];
        assert_eq!(t, "table");

        let result: Encrypted = serde_json::from_value(value).unwrap();

        if let Encrypted::Ciphertext { identifier, .. } = result {
            assert_eq!(expected, identifier);
        } else {
            panic!("Expected Encrypted::Ciphertext");
        }
    }
}
