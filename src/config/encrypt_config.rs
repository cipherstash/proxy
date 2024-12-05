use cipherstash_config::{
    column::{Index, IndexType, TokenFilter, Tokenizer},
    errors::ConfigError as DatasetConfigError,
    ColumnConfig, ColumnType, DatasetConfig, TableConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{ConfigError, Error};

pub type TableName = String;
pub type ColumnName = String;
pub type Tables = HashMap<TableName, Table>;
pub type Table = HashMap<ColumnName, Column>;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct EncryptConfig {
    #[serde(rename = "v")]
    pub version: u32,
    pub tables: Tables,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, PartialEq)]
pub struct Column {
    #[serde(default)]
    cast_as: CastAs,
    #[serde(default)]
    indexes: Indexes,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CastAs {
    BigInt,
    Boolean,
    Date,
    Real,
    Double,
    Int,
    SmallInt,
    #[default]
    Text,
    #[serde(rename = "jsonb")]
    JsonB,
}

// TODO: list instead of struct here/object in DB?
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
pub struct Indexes {
    #[serde(rename = "ore")]
    ore_index: Option<OreIndexOpts>,
    #[serde(rename = "unique")]
    unique_index: Option<UniqueIndexOpts>,
    #[serde(rename = "match")]
    match_index: Option<MatchIndexOpts>,
    #[serde(rename = "ste_vec")]
    ste_vec_index: Option<SteVecIndexOpts>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct OreIndexOpts {}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct MatchIndexOpts {
    #[serde(default = "default_tokenizer")]
    tokenizer: Tokenizer,
    #[serde(default)]
    token_filters: Vec<TokenFilter>,
    #[serde(default = "default_k")]
    k: usize,
    #[serde(default = "default_m")]
    m: usize,
    #[serde(default)]
    include_original: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct SteVecIndexOpts {
    prefix: String,
}

fn default_tokenizer() -> Tokenizer {
    Tokenizer::Standard
}

fn default_k() -> usize {
    6
}

fn default_m() -> usize {
    2048
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct UniqueIndexOpts {
    #[serde(default)]
    token_filters: Vec<TokenFilter>,
}

impl From<CastAs> for ColumnType {
    fn from(value: CastAs) -> Self {
        match value {
            CastAs::BigInt => ColumnType::BigInt,
            CastAs::SmallInt => ColumnType::SmallInt,
            CastAs::Int => ColumnType::Int,
            CastAs::Boolean => ColumnType::Boolean,
            CastAs::Date => ColumnType::Date,
            CastAs::Real | CastAs::Double => ColumnType::Float,
            CastAs::Text => ColumnType::Utf8Str,
            CastAs::JsonB => ColumnType::JsonB,
        }
    }
}

impl EncryptConfig {
    pub fn from_str(data: &str) -> Result<Self, Error> {
        let config = serde_json::from_str(&data).map_err(|e| ConfigError::Parse(e))?;
        Ok(config)
    }

    pub fn to_config_map(self) -> HashMap<String, ColumnConfig> {
        let mut map = HashMap::new();
        for (table_name, columns) in self.tables.into_iter() {
            for (name, column) in columns.into_iter() {
                let column_config = column.to_column_config(&name);
                let key = format!("{}.{}", table_name, name);
                map.insert(key, column_config);
            }
        }
        map
    }
}

impl Column {
    pub fn to_column_config(self, name: &str) -> ColumnConfig {
        let mut config = ColumnConfig::build(name).casts_as(self.cast_as.into());

        if self.indexes.ore_index.is_some() {
            config = config.add_index(Index::new_ore());
        }

        if let Some(opts) = self.indexes.match_index {
            config = config.add_index(Index::new(IndexType::Match {
                tokenizer: opts.tokenizer,
                token_filters: opts.token_filters,
                k: opts.k,
                m: opts.m,
                include_original: opts.include_original,
            }));
        }

        if let Some(opts) = self.indexes.unique_index {
            config = config.add_index(Index::new(IndexType::Unique {
                token_filters: opts.token_filters,
            }))
        }

        if let Some(SteVecIndexOpts { prefix }) = self.indexes.ste_vec_index {
            config = config.add_index(Index::new(IndexType::SteVec { prefix }))
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use rustls::crypto::hash::Hash;
    use serde_json::json;

    use super::*;

    fn parse(json: serde_json::Value) -> HashMap<String, ColumnConfig> {
        serde_json::from_value::<EncryptConfig>(json)
            .map(|config| config.to_config_map())
            .expect("ok")
    }

    #[test]
    fn column_with_empty_options_gets_defaults() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {}
                }
            }
        });

        let encrypt_config = parse(json);

        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(column.cast_type, ColumnType::Utf8Str);
        assert!(column.indexes.is_empty());
    }

    #[test]
    fn can_parse_column_with_cast_as() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "favourite_int": {
                        "cast_as": "int"
                    }
                }
            }
        });

        let encrypt_config = parse(json);

        let column = encrypt_config
            .get("users.favourite_int")
            .expect("column exists");

        assert_eq!(column.cast_type, ColumnType::Int);
        assert_eq!(column.name, "favourite_int");
        assert!(column.indexes.is_empty());
    }

    #[test]
    fn can_parse_empty_indexes() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {}
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert!(column.indexes.is_empty());
    }

    #[test]
    fn can_parse_ore_index() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "ore": {}
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(column.indexes[0].index_type, IndexType::Ore);
    }

    #[test]
    fn can_parse_unique_index_with_defaults() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "unique": {}
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Unique {
                token_filters: vec![]
            }
        );
    }

    #[test]
    fn can_parse_unique_index_with_token_filter() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "unique": {
                                "token_filters": [
                                    {
                                        "kind": "downcase"
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Unique {
                token_filters: vec![TokenFilter::Downcase]
            }
        );
    }

    #[test]
    fn can_parse_match_index_with_defaults() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "match": {}
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Match {
                tokenizer: Tokenizer::Standard,
                token_filters: vec![],
                k: 6,
                m: 2048,
                include_original: false
            }
        );
    }

    #[test]
    fn can_parse_match_index_with_all_opts_set() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "indexes": {
                            "match": {
                                "tokenizer": {
                                    "kind": "ngram",
                                    "token_length": 3,
                                },
                                "token_filters": [
                                    {
                                        "kind": "downcase"
                                    }
                                ],
                                "k": 8,
                                "m": 1024,
                                "include_original": true
                            }
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config.get("users.email").expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::Match {
                tokenizer: Tokenizer::Ngram { token_length: 3 },
                token_filters: vec![TokenFilter::Downcase],
                k: 8,
                m: 1024,
                include_original: true
            }
        );
    }

    #[test]
    fn can_parse_ste_vec_index() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "event_data": {
                        "indexes": {
                            "ste_vec": {
                                "prefix": "event-data"
                            }
                        }
                    }
                }
            }
        });

        let encrypt_config = parse(json);
        let column = encrypt_config
            .get("users.event_data")
            .expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::SteVec {
                prefix: "event-data".into()
            },
        );
    }
}
