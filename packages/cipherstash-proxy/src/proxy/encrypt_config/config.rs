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

    #[test]
    fn config_map_preserves_table_and_column_names() {
        let json = json!({
            "v": 1,
            "tables": {
                "my_schema.users": {
                    "email_address": {
                        "cast_as": "text",
                        "indexes": { "unique": {} }
                    }
                }
            }
        });

        let config = parse(json);

        let ident = Identifier::new("my_schema.users", "email_address");
        let column = config.get(&ident).expect("column exists");
        assert_eq!(column.name, "email_address");
        assert_eq!(column.cast_type, ColumnType::Text);
    }

    #[test]
    fn config_map_handles_multiple_tables() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": { "cast_as": "text" }
                },
                "orders": {
                    "total": { "cast_as": "int" }
                }
            }
        });

        let config = parse(json);

        assert_eq!(config.len(), 2);

        let email = config
            .get(&Identifier::new("users", "email"))
            .expect("users.email exists");
        assert_eq!(email.cast_type, ColumnType::Text);

        let total = config
            .get(&Identifier::new("orders", "total"))
            .expect("orders.total exists");
        assert_eq!(total.cast_type, ColumnType::Int);
    }

    #[test]
    fn invalid_config_returns_error() {
        let json = json!({
            "v": 1,
            "tables": {
                "users": {
                    "email": {
                        "cast_as": "text",
                        "indexes": {
                            "ste_vec": { "prefix": "test" }
                        }
                    }
                }
            }
        });

        let config: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
        let result = config.into_config_map();
        assert!(
            result.is_err(),
            "ste_vec on text column should fail validation"
        );
    }

    #[test]
    fn real_eql_config_produces_correct_encrypt_config() {
        let json = json!({
            "v": 1,
            "tables": {
                "encrypted": {
                    "encrypted_text": {
                        "cast_as": "text",
                        "indexes": { "unique": {}, "match": {}, "ore": {} }
                    },
                    "encrypted_bool": {
                        "cast_as": "boolean",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int2": {
                        "cast_as": "small_int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int4": {
                        "cast_as": "int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_int8": {
                        "cast_as": "big_int",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_float8": {
                        "cast_as": "double",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_date": {
                        "cast_as": "date",
                        "indexes": { "unique": {}, "ore": {} }
                    },
                    "encrypted_jsonb": {
                        "cast_as": "jsonb",
                        "indexes": {
                            "ste_vec": { "prefix": "encrypted/encrypted_jsonb" }
                        }
                    },
                    "encrypted_jsonb_filtered": {
                        "cast_as": "jsonb",
                        "indexes": {
                            "ste_vec": {
                                "prefix": "encrypted/encrypted_jsonb_filtered",
                                "term_filters": [{ "kind": "downcase" }]
                            }
                        }
                    }
                }
            }
        });

        let config = parse(json);

        // All 9 columns present with correct Identifiers
        assert_eq!(config.len(), 9);

        // Verify legacy type aliases map correctly
        let float_col = config
            .get(&Identifier::new("encrypted", "encrypted_float8"))
            .unwrap();
        assert_eq!(float_col.cast_type, ColumnType::Float);

        let jsonb_col = config
            .get(&Identifier::new("encrypted", "encrypted_jsonb"))
            .unwrap();
        assert_eq!(jsonb_col.cast_type, ColumnType::Json);

        // Verify index counts
        let text_col = config
            .get(&Identifier::new("encrypted", "encrypted_text"))
            .unwrap();
        assert_eq!(text_col.indexes.len(), 3);

        let bool_col = config
            .get(&Identifier::new("encrypted", "encrypted_bool"))
            .unwrap();
        assert_eq!(bool_col.indexes.len(), 2);

        let jsonb_filtered = config
            .get(&Identifier::new("encrypted", "encrypted_jsonb_filtered"))
            .unwrap();
        assert_eq!(jsonb_filtered.indexes.len(), 1);
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let json = json!({
            "v": 1,
            "tables": "not a map"
        });

        let result = serde_json::from_value::<CanonicalEncryptionConfig>(json);
        assert!(result.is_err());
    }
}
