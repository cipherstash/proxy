use cipherstash_client::{
    encryption::SteVec,
    zerokms::{encrypted_record, EncryptedRecord},
};
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "k")]
pub enum Encrypted {
    #[serde(rename = "ct")]
    Ciphertext {
        #[serde(rename = "c", with = "encrypted_record::formats::mp_base85")]
        ciphertext: EncryptedRecord,
        #[serde(rename = "o")]
        ore_index: Option<String>,
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
