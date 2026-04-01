# Migrate Proxy to Canonical Encryption Config — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use cipherpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the proxy's local `CastAs` enum and `ColumnEncryptionConfig` parser, replacing them with `CanonicalEncryptionConfig` from the `cipherstash-config` crate.

**Architecture:** The proxy currently has its own JSON config parser in `encrypt_config/config.rs` (~490 lines) that duplicates what `cipherstash-config` provides. We replace it with the canonical parser, keeping only the `EncryptConfig` wrapper and `EncryptConfigManager` which handle proxy-specific concerns (Arc-wrapped config, background reload).

**Tech Stack:** Rust, serde, serde_json, cipherstash-config

**Prerequisite:** The canonical config work in `cipherstash-suite` (CIP-2871) must be completed first — specifically, `CanonicalEncryptionConfig`, `PlaintextType`, `Identifier`, and `into_config_map()` must exist in `cipherstash-config`.

**Design doc:** `~/cipherstash/cipherstash-suite/docs/plans/2026-04-01-canonical-encryption-config-design.md`

---

### Task 1: Add `cipherstash-config` dependency

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `packages/cipherstash-proxy/Cargo.toml`

**Step 1: Add to workspace dependencies**

In the root `Cargo.toml`, add `cipherstash-config` to `[workspace.dependencies]`. Match the version/source used for `cipherstash-client`.

**Step 2: Add to cipherstash-proxy package**

In `packages/cipherstash-proxy/Cargo.toml`, add:

```toml
cipherstash-config = { workspace = true }
```

**Step 3: Verify it compiles**

Run: `cargo check -p cipherstash-proxy`
Expected: Clean build

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock packages/cipherstash-proxy/Cargo.toml
git commit --no-gpg-sign -m "chore: add cipherstash-config dependency to cipherstash-proxy"
```

---

### Task 2: Replace `ColumnEncryptionConfig` with `CanonicalEncryptionConfig` in the manager

**Files:**
- Modify: `packages/cipherstash-proxy/src/proxy/encrypt_config/manager.rs`

**Step 1: Update imports**

Replace the import of local `ColumnEncryptionConfig`:

```rust
// Before
use super::config::ColumnEncryptionConfig;

// After
use cipherstash_config::CanonicalEncryptionConfig;
```

**Step 2: Update `load_encrypt_config` function**

The function currently does (around line 216):

```rust
let encrypt_config: ColumnEncryptionConfig = serde_json::from_value(json_value)?;
let encrypt_config = EncryptConfig::new_from_config(encrypt_config.into_config_map());
```

Change to:

```rust
let encrypt_config: CanonicalEncryptionConfig = serde_json::from_value(json_value)
    .map_err(|e| /* map to existing error type */)?;
let config_map = encrypt_config.into_config_map()
    .map_err(|e| /* map ConfigError to proxy Error */)?;
let encrypt_config = EncryptConfig::new_from_config(config_map);
```

Note: The canonical `into_config_map()` returns `Result<HashMap<Identifier, ColumnConfig>, ConfigError>` (fallible, with validation) while the proxy's was infallible. You'll need to handle the `Result` — map `ConfigError` to the proxy's error type.

Also note: The canonical `Identifier` is from `cipherstash_config::Identifier`, not `cipherstash_client::eql::Identifier`. Check that `EncryptConfig::new_from_config` and `EncryptConfig::get_column_config` use the same `Identifier` type. If they differ, update `EncryptConfig` to use the canonical `Identifier`.

**Step 3: Run tests**

Run: `cargo test -p cipherstash-proxy --lib -- encrypt_config`
Expected: All pass

**Step 4: Commit**

```bash
git add packages/cipherstash-proxy/src/proxy/encrypt_config/manager.rs
git commit --no-gpg-sign -m "refactor: use CanonicalEncryptionConfig in EncryptConfigManager"
```

---

### Task 3: Remove local config types

**Files:**
- Modify: `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs`

**Step 1: Delete local types**

Remove the following from `config.rs`:
- `ColumnEncryptionConfig` struct
- `Tables` struct and its `IntoIterator` impl
- `Table` struct and its `IntoIterator` impl
- `Column` struct
- `CastAs` enum
- `From<CastAs> for ColumnType` impl
- `OreIndexOpts`, `MatchIndexOpts`, `SteVecIndexOpts`, `UniqueIndexOpts` structs
- `Indexes` struct
- `FromStr for ColumnEncryptionConfig` impl
- `ColumnEncryptionConfig::into_config_map` method
- `Column::into_column_config` method
- All default functions (`default_tokenizer`, `default_k`, `default_m`, `default_array_index_mode`)

This should remove ~200 lines of code. What remains in `config.rs` (if anything) depends on whether the proxy has any config types not covered by the canonical module.

**Step 2: Update `mod.rs` if needed**

If `config.rs` is now empty or only has tests, update `packages/cipherstash-proxy/src/proxy/encrypt_config/mod.rs` accordingly.

**Step 3: Run tests**

Run: `cargo test -p cipherstash-proxy --lib -- encrypt_config`
Expected: All pass

Run: `cargo clippy --no-deps --tests --all-features --all-targets -p cipherstash-proxy -- -D warnings`
Expected: No warnings

**Step 4: Commit**

```bash
git add packages/cipherstash-proxy/src/proxy/encrypt_config/
git commit --no-gpg-sign -m "refactor: remove local CastAs and ColumnEncryptionConfig, use canonical types"
```

---

### Task 4: Update tests to use canonical types

**Files:**
- Modify: `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs` (test module)

**Step 1: Migrate tests**

The existing tests (lines 210-489) test JSON parsing of the local types. Rewrite them to test via `CanonicalEncryptionConfig` and `into_config_map()`. Key tests to preserve:

- `column_with_empty_options_gets_defaults` — empty column defaults to `Text` with no indexes
- `can_parse_column_with_cast_as` — `"cast_as": "int"` parses correctly
- `can_parse_ore_index` — ORE index deserializes
- `can_parse_unique_index_with_token_filter` — unique with downcase filter
- `can_parse_match_index_with_defaults` — match index gets k=6, m=2048, Standard tokenizer
- `can_parse_match_index_with_all_opts_set` — custom match options
- `can_parse_ste_vec_index` — STE vec with prefix and array_index_mode

Each test should:
1. Build JSON with `serde_json::json!`
2. Deserialize to `CanonicalEncryptionConfig`
3. Call `into_config_map()`
4. Assert on the resulting `ColumnConfig`

Example:

```rust
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

    let config: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
    let map = config.into_config_map().unwrap();

    let id = Identifier::new("users", "email");
    let col = map.get(&id).unwrap();
    assert_eq!(col.cast_type, ColumnType::Text);
    assert!(col.indexes.is_empty());
}
```

Add a backwards-compat test:

```rust
#[test]
fn it_accepts_old_cast_as_jsonb() {
    let json = json!({
        "v": 1,
        "tables": {
            "events": {
                "data": {
                    "cast_as": "jsonb",
                    "indexes": {
                        "ste_vec": { "prefix": "test" }
                    }
                }
            }
        }
    });

    let config: CanonicalEncryptionConfig = serde_json::from_value(json).unwrap();
    let map = config.into_config_map().unwrap();
    let id = Identifier::new("events", "data");
    let col = map.get(&id).unwrap();
    assert_eq!(col.cast_type, ColumnType::Json);
}
```

**Step 2: Run tests**

Run: `cargo test -p cipherstash-proxy --lib -- encrypt_config`
Expected: All pass

**Step 3: Commit**

```bash
git add packages/cipherstash-proxy/src/proxy/encrypt_config/
git commit --no-gpg-sign -m "test: migrate encrypt_config tests to use canonical types"
```

---

### Task 5: Full build and test verification

**Files:** None (verification only)

**Step 1: Workspace clippy**

Run: `cargo clippy --no-deps --tests --all-features --all-targets --workspace -- -D warnings`
Expected: No warnings

**Step 2: Unit tests**

Run: `cargo test --workspace --all-features --lib`
Expected: All pass

**Step 3: Integration tests (if environment available)**

Run: `mise run test:local:integration` (requires PostgreSQL running)
Expected: All pass

**Step 4: If any failures, fix and commit**

```bash
git add -u
git commit --no-gpg-sign -m "fix: resolve build issues from canonical config migration"
```
