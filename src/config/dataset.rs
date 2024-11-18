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
pub struct JsonDatasetConfig {
    #[serde(rename = "v")]
    version: u32,
    tables: Tables,
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

impl TryFrom<JsonDatasetConfig> for DatasetConfig {
    type Error = DatasetConfigError;

    fn try_from(value: JsonDatasetConfig) -> Result<Self, Self::Error> {
        let mut dataset_config = DatasetConfig::init();

        for (table_name, columns) in value.tables.into_iter() {
            let mut table_config = TableConfig::new(table_name.as_str())?;

            for (column_name, column) in columns.into_iter() {
                let mut column_config =
                    ColumnConfig::build(column_name).casts_as(column.cast_as.into());

                if column.indexes.ore_index.is_some() {
                    column_config = column_config.add_index(Index::new_ore());
                }

                if let Some(opts) = column.indexes.match_index {
                    column_config = column_config.add_index(Index::new(IndexType::Match {
                        tokenizer: opts.tokenizer,
                        token_filters: opts.token_filters,
                        k: opts.k,
                        m: opts.m,
                        include_original: opts.include_original,
                    }));
                }

                if let Some(opts) = column.indexes.unique_index {
                    column_config = column_config.add_index(Index::new(IndexType::Unique {
                        token_filters: opts.token_filters,
                    }))
                }

                if let Some(SteVecIndexOpts { prefix }) = column.indexes.ste_vec_index {
                    column_config =
                        column_config.add_index(Index::new(IndexType::SteVec { prefix }))
                }

                table_config = table_config.add_column(column_config)?;
            }

            dataset_config = dataset_config.add_table(table_config)?;
        }

        Ok(dataset_config)
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(json: serde_json::Value) -> DatasetConfig {
        serde_json::from_value::<JsonDatasetConfig>(json)
            .expect("failed to convert JSON to PostgresEncryptConfig")
            .try_into()
            .expect("failed to convert PostgresEncryptConfig to DatasetConfig")
    }

    #[test]
    fn can_parse_empty_tables() {
        let json = json!({
            "v": 1,
            "tables": {}
        });

        let dataset_config = parse(json);

        assert!(dataset_config.tables.is_empty());
    }

    #[test]
    fn can_parse_table_with_empty_columns() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {}
            }
        });

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");

        assert!(dataset_config.tables[0].fields.is_empty());
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");

        assert_eq!(dataset_config.tables[0].fields.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");
        assert_eq!(
            dataset_config.tables[0].fields[0].cast_type,
            ColumnType::Utf8Str
        );
        assert!(dataset_config.tables[0].fields[0].indexes.is_empty());
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");

        assert_eq!(dataset_config.tables[0].fields.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "favourite_int");
        assert_eq!(
            dataset_config.tables[0].fields[0].cast_type,
            ColumnType::Int
        );
        assert!(dataset_config.tables[0].fields[0].indexes.is_empty());
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");

        assert_eq!(dataset_config.tables[0].fields.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert!(dataset_config.tables[0].fields[0].indexes.is_empty());
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
            IndexType::Ore
        );
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "email");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
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

        let dataset_config = parse(json);

        assert_eq!(dataset_config.tables.len(), 1);
        assert_eq!(dataset_config.tables[0].path.as_string(), "users");
        assert_eq!(dataset_config.tables[0].fields.len(), 1);

        assert_eq!(dataset_config.tables[0].fields[0].indexes.len(), 1);
        assert_eq!(dataset_config.tables[0].fields[0].name, "event_data");

        assert_eq!(
            dataset_config.tables[0].fields[0].indexes[0].index_type,
            IndexType::SteVec {
                prefix: "event-data".into()
            },
        );
    }
}
