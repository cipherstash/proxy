# Proxy Backwards Compatibility Tests for Canonical Config Migration

> **For Claude:** REQUIRED SUB-SKILL: Use cipherpowers:executing-plans to implement this plan task-by-task.

**Goal:** Verify the proxy's integration pipeline from JSON → `CanonicalEncryptionConfig` → `EncryptConfig` works correctly, including Identifier conversion, error handling, and ColumnType mapping.

**Tech Stack:** Rust, serde_json, cipherstash-config, cipherstash-client

---

### Task 1: Test Identifier bridging in EncryptConfig

**File:** `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs` (test module)

The `load_encrypt_config` function converts `cipherstash_config::Identifier` → `cipherstash_client::eql::Identifier`. Test that this preserves table/column names correctly.

Add these tests to the existing test module:

```rust
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
    assert!(config.contains_key(&Identifier::new("users", "email")));
    assert!(config.contains_key(&Identifier::new("orders", "total")));
}
```

**Verify:** `cargo test -p cipherstash-proxy --lib -- encrypt_config`

---

### Task 2: Test ColumnType mapping through column_type_to_postgres_type

**File:** `packages/cipherstash-proxy/src/postgresql/context/column.rs`

The rename from `Utf8Str` → `Text` and `JsonB` → `Json` must produce the same PostgreSQL types. Add tests to verify the mapping.

Add a test module to `column.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use eql_mapper::EqlTermVariant;

    #[test]
    fn text_column_maps_to_postgres_text() {
        assert_eq!(
            column_type_to_postgres_type(&ColumnType::Text, EqlTermVariant::Full),
            postgres_types::Type::TEXT
        );
    }

    #[test]
    fn json_column_maps_to_postgres_jsonb() {
        assert_eq!(
            column_type_to_postgres_type(&ColumnType::Json, EqlTermVariant::Full),
            postgres_types::Type::JSONB
        );
    }

    #[test]
    fn json_accessor_maps_to_postgres_text() {
        assert_eq!(
            column_type_to_postgres_type(&ColumnType::Json, EqlTermVariant::JsonAccessor),
            postgres_types::Type::TEXT
        );
    }

    #[test]
    fn all_column_types_have_postgres_mapping() {
        let types = vec![
            ColumnType::Boolean,
            ColumnType::BigInt,
            ColumnType::BigUInt,
            ColumnType::Date,
            ColumnType::Decimal,
            ColumnType::Float,
            ColumnType::Int,
            ColumnType::SmallInt,
            ColumnType::Timestamp,
            ColumnType::Text,
            ColumnType::Json,
        ];

        for ct in types {
            // Should not panic
            let _ = column_type_to_postgres_type(&ct, EqlTermVariant::Full);
        }
    }
}
```

**Verify:** `cargo test -p cipherstash-proxy --lib -- context::column`

---

### Task 3: Test error propagation for invalid configs

**File:** `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs` (test module)

The canonical `into_config_map()` can now return errors (e.g., ste_vec on non-JSON column). Verify the error surfaces correctly through the proxy's error types.

```rust
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
    assert!(result.is_err(), "ste_vec on text column should fail validation");
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
```

**Verify:** `cargo test -p cipherstash-proxy --lib -- encrypt_config`

---

### Task 4: Test real integration schema config shape

**File:** `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs` (test module)

Use the same fixture from the cipherstash-config plan — the JSON shape matching the proxy's integration test schema. Verify the full pipeline including Identifier conversion.

```rust
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
    let float_col = config.get(&Identifier::new("encrypted", "encrypted_float8")).unwrap();
    assert_eq!(float_col.cast_type, ColumnType::Float);

    let jsonb_col = config.get(&Identifier::new("encrypted", "encrypted_jsonb")).unwrap();
    assert_eq!(jsonb_col.cast_type, ColumnType::Json);

    // Verify index counts
    let text_col = config.get(&Identifier::new("encrypted", "encrypted_text")).unwrap();
    assert_eq!(text_col.indexes.len(), 3);

    let bool_col = config.get(&Identifier::new("encrypted", "encrypted_bool")).unwrap();
    assert_eq!(bool_col.indexes.len(), 2);

    let jsonb_filtered = config.get(&Identifier::new("encrypted", "encrypted_jsonb_filtered")).unwrap();
    assert_eq!(jsonb_filtered.indexes.len(), 1);
}
```

**Verify:** `cargo test -p cipherstash-proxy --lib -- encrypt_config`

---

### Task 5: Full verification

Run the complete test suite:

```bash
cargo clippy --no-deps --tests --all-features --all-targets -p cipherstash-proxy -- -D warnings
cargo test -p cipherstash-proxy --lib
```

All tests must pass, zero clippy warnings.
