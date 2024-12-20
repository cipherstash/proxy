use cipherstash_client::zerokms::{encrypted_record, EncryptedRecord};
use serde::{Deserialize, Serialize, Serializer};
use sqlparser::ast::Ident;

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
    pub table: String,
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
}

impl From<(&Ident, &Ident)> for Identifier {
    fn from((table, column): (&Ident, &Ident)) -> Self {
        Self {
            table: table.to_string(),
            column: column.to_string(),
        }
    }
}

// #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
// pub struct Identifier {
//     #[serde(
//         rename = "t",
//         deserialize_with = "ident_de",
//         serialize_with = "ident_se"
//     )]
//     pub table: Ident,
//     #[serde(
//         rename = "c",
//         deserialize_with = "ident_de",
//         serialize_with = "ident_se"
//     )]
//     pub column: Ident,
// }

// impl Identifier {
//     pub fn new<S>(table: S, column: S) -> Self
//     where
//         S: Into<String>,
//     {
//         let table = Ident::with_quote('"', table);
//         let column = Ident::with_quote('"', column);

//         Self { table, column }
//     }
// }

// impl From<(&Ident, &Ident)> for Identifier {
//     fn from((table, column): (&Ident, &Ident)) -> Self {
//         Self {
//             table: table.to_owned(),
//             column: column.to_owned(),
//         }
//     }
// }

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

fn ident_de<'de, D>(deserializer: D) -> Result<Ident, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Ident::with_quote('"', s))
}

fn ident_se<S>(ident: &Ident, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = ident.to_string();
    serializer.serialize_str(&s)
}
