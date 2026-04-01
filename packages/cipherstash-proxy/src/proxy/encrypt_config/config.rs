#[cfg(test)]
mod tests {
    use cipherstash_client::eql::Identifier;
    use cipherstash_client::schema::ColumnConfig;
    use cipherstash_config::column::{ArrayIndexMode, IndexType, TokenFilter, Tokenizer};
    use cipherstash_config::{CanonicalEncryptionConfig, ColumnType};
    use serde_json::json;
    use std::collections::HashMap;

    fn parse(json: serde_json::Value) -> HashMap<Identifier, ColumnConfig> {
        let config: CanonicalEncryptionConfig =
            serde_json::from_value(json).expect("Failed to parse config");
        config
            .into_config_map()
            .expect("Failed to build config map")
            .into_iter()
            .map(|(id, col)| (Identifier::new(id.table, id.column), col))
            .collect()
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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

        assert_eq!(column.cast_type, ColumnType::Text);
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

        let ident = Identifier::new("users", "favourite_int");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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

        let ident = Identifier::new("users", "email");

        let column = encrypt_config.get(&ident).expect("column exists");

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
                        "cast_as": "jsonb",
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

        let ident = Identifier::new("users", "event_data");

        let column = encrypt_config.get(&ident).expect("column exists");

        assert_eq!(
            column.indexes[0].index_type,
            IndexType::SteVec {
                prefix: "event-data".into(),
                term_filters: vec![],
                array_index_mode: ArrayIndexMode::ALL,
            },
        );
    }
}
