use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plaintext {
    #[serde(rename = "p")]
    plaintext: String,
    #[serde(rename = "i")]
    identifier: Identifier,
    #[serde(rename = "v")]
    version: u16,
    #[serde(rename = "q")]
    for_query: Option<ForQuery>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub struct Encrypted {
    v: usize,
    cfg: usize,
    knd: String,
}
