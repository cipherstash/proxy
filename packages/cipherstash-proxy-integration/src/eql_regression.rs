//! EQL Regression Tests
//!
//! These tests verify backwards compatibility with data encrypted by previous versions
//! of the proxy. This is critical for production deployments where existing data must
//! remain readable after proxy upgrades.
//!
//! ## How to use these tests:
//!
//! 1. **Generate fixtures from main branch:**
//!    ```
//!    git checkout main
//!    CS_GENERATE_EQL_FIXTURES=1 cargo nextest run -p cipherstash-proxy-integration eql_regression::generate
//!    ```
//!
//! 2. **Run regression tests on new branch:**
//!    ```
//!    git checkout <your-branch>
//!    cargo nextest run -p cipherstash-proxy-integration eql_regression::regression
//!    ```
//!
//! The fixtures are stored in `tests/fixtures/eql_regression/` and should be committed
//! to the repository after being generated from main.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;
    use tokio_postgres::types::ToSql;

    const FIXTURES_DIR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/eql_regression"
    );

    /// Represents a captured EQL ciphertext for regression testing
    #[derive(Debug, Serialize, Deserialize)]
    struct EqlFixture {
        /// Description of what this fixture tests
        description: String,
        /// The original plaintext value (for verification)
        plaintext: Value,
        /// The encrypted ciphertext as stored in the database
        ciphertext: String,
        /// The data type (text, jsonb, int4, etc.)
        data_type: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct FixtureSet {
        /// Version of the proxy that generated these fixtures
        proxy_version: String,
        /// Git commit hash when fixtures were generated
        git_commit: String,
        /// The fixtures
        fixtures: Vec<EqlFixture>,
    }

    fn get_database_port() -> u16 {
        std::env::var("CS_DATABASE__PORT")
            .map(|s| s.parse().unwrap())
            .unwrap_or(5617) // Default to TLS port
    }

    /// Insert data via proxy and return the encrypted ciphertext from the database
    async fn capture_encrypted_ciphertext(
        column: &str,
        plaintext: &(dyn ToSql + Sync),
        plaintext_json: Value,
    ) -> EqlFixture {
        let id = random_id();

        // Insert via proxy (will encrypt)
        let proxy_client = connect_with_tls(PROXY).await;
        let sql = format!("INSERT INTO encrypted (id, {column}) VALUES ($1, $2)");
        proxy_client
            .execute(&sql, &[&id, plaintext])
            .await
            .expect("Failed to insert via proxy");

        // Read encrypted value directly from database (bypassing proxy)
        let db_port = get_database_port();
        let db_client = connect_with_tls(db_port).await;
        let sql = format!("SELECT {column}::text FROM encrypted WHERE id = $1");
        let rows = db_client
            .query(&sql, &[&id])
            .await
            .expect("Failed to query directly");

        let ciphertext: String = rows[0].get(0);

        EqlFixture {
            description: format!("Encrypted {column}"),
            plaintext: plaintext_json,
            ciphertext,
            data_type: column.replace("encrypted_", ""),
        }
    }

    /// Insert pre-encrypted data directly into the database
    async fn insert_encrypted_directly(id: i64, column: &str, ciphertext: &str) {
        let db_port = get_database_port();
        let db_client = connect_with_tls(db_port).await;

        // Insert the raw ciphertext directly, casting from text to eql_v2_encrypted
        let sql = format!("INSERT INTO encrypted (id, {column}) VALUES ($1, $2::eql_v2_encrypted)");
        db_client
            .execute(&sql, &[&id, &ciphertext])
            .await
            .expect("Failed to insert encrypted data directly");
    }

    /// Read and decrypt data via proxy
    async fn decrypt_via_proxy<T>(id: i64, column: &str) -> T
    where
        T: for<'a> tokio_postgres::types::FromSql<'a>,
    {
        let proxy_client = connect_with_tls(PROXY).await;
        let sql = format!("SELECT {column} FROM encrypted WHERE id = $1");
        let rows = proxy_client
            .query(&sql, &[&id])
            .await
            .expect("Failed to query via proxy");

        rows[0].get(0)
    }

    fn fixtures_path() -> PathBuf {
        PathBuf::from(FIXTURES_DIR)
    }

    fn load_fixtures() -> Option<FixtureSet> {
        let path = fixtures_path().join("fixtures.json");
        if path.exists() {
            let content = fs::read_to_string(&path).expect("Failed to read fixtures file");
            Some(serde_json::from_str(&content).expect("Failed to parse fixtures"))
        } else {
            None
        }
    }

    fn save_fixtures(fixtures: &FixtureSet) {
        let path = fixtures_path();
        fs::create_dir_all(&path).expect("Failed to create fixtures directory");

        let content = serde_json::to_string_pretty(fixtures).expect("Failed to serialize fixtures");
        fs::write(path.join("fixtures.json"), content).expect("Failed to write fixtures file");
    }

    /// Generate fixtures from the current proxy version.
    /// Run this on the main branch to create baseline fixtures.
    ///
    /// Set CS_GENERATE_EQL_FIXTURES=1 to enable fixture generation.
    #[tokio::test]
    async fn generate_fixtures() {
        if std::env::var("CS_GENERATE_EQL_FIXTURES").is_err() {
            println!("Skipping fixture generation. Set CS_GENERATE_EQL_FIXTURES=1 to generate.");
            return;
        }

        trace();
        clear().await;

        let mut fixtures = Vec::new();

        // Text
        let text_value = "regression test text";
        fixtures.push(
            capture_encrypted_ciphertext(
                "encrypted_text",
                &text_value,
                Value::String(text_value.to_string()),
            )
            .await,
        );

        // Integer types
        let int2_value: i16 = 42;
        fixtures.push(
            capture_encrypted_ciphertext(
                "encrypted_int2",
                &int2_value,
                Value::Number(int2_value.into()),
            )
            .await,
        );

        let int4_value: i32 = 12345;
        fixtures.push(
            capture_encrypted_ciphertext(
                "encrypted_int4",
                &int4_value,
                Value::Number(int4_value.into()),
            )
            .await,
        );

        let int8_value: i64 = 9876543210;
        fixtures.push(
            capture_encrypted_ciphertext(
                "encrypted_int8",
                &int8_value,
                Value::Number(int8_value.into()),
            )
            .await,
        );

        // Float
        let float_value: f64 = std::f64::consts::PI;
        fixtures.push(
            capture_encrypted_ciphertext(
                "encrypted_float8",
                &float_value,
                serde_json::json!(float_value),
            )
            .await,
        );

        // Boolean
        let bool_value = true;
        fixtures.push(
            capture_encrypted_ciphertext("encrypted_bool", &bool_value, Value::Bool(bool_value))
                .await,
        );

        // JSONB - simple object
        let jsonb_value = serde_json::json!({
            "string": "hello",
            "number": 42,
            "nested": {
                "key": "value"
            },
            "array_string": ["a", "b", "c"],
            "array_number": [1, 2, 3]
        });
        fixtures.push(
            capture_encrypted_ciphertext("encrypted_jsonb", &jsonb_value, jsonb_value.clone())
                .await,
        );

        // Get git commit for documentation
        let git_commit = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let fixture_set = FixtureSet {
            proxy_version: env!("CARGO_PKG_VERSION").to_string(),
            git_commit,
            fixtures,
        };

        save_fixtures(&fixture_set);
        println!(
            "Generated {} fixtures at {:?}",
            fixture_set.fixtures.len(),
            fixtures_path()
        );
    }

    /// Regression test: verify that data encrypted by a previous proxy version
    /// can still be decrypted by the current version.
    #[tokio::test]
    async fn regression_decrypt_legacy_text() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            println!("No fixtures found. Run generate_fixtures on main branch first.");
            println!("Set CS_GENERATE_EQL_FIXTURES=1 and run on main branch.");
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "text" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_text", &fixture.ciphertext).await;

            let decrypted: String = decrypt_via_proxy(id, "encrypted_text").await;
            let expected = fixture.plaintext.as_str().unwrap();

            assert_eq!(
                decrypted, expected,
                "Failed to decrypt legacy text. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_int2() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "int2" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_int2", &fixture.ciphertext).await;

            let decrypted: i16 = decrypt_via_proxy(id, "encrypted_int2").await;
            let expected = fixture.plaintext.as_i64().unwrap() as i16;

            assert_eq!(
                decrypted, expected,
                "Failed to decrypt legacy int2. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_int4() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "int4" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_int4", &fixture.ciphertext).await;

            let decrypted: i32 = decrypt_via_proxy(id, "encrypted_int4").await;
            let expected = fixture.plaintext.as_i64().unwrap() as i32;

            assert_eq!(
                decrypted, expected,
                "Failed to decrypt legacy int4. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_int8() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "int8" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_int8", &fixture.ciphertext).await;

            let decrypted: i64 = decrypt_via_proxy(id, "encrypted_int8").await;
            let expected = fixture.plaintext.as_i64().unwrap();

            assert_eq!(
                decrypted, expected,
                "Failed to decrypt legacy int8. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_float8() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "float8" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_float8", &fixture.ciphertext).await;

            let decrypted: f64 = decrypt_via_proxy(id, "encrypted_float8").await;
            let expected = fixture.plaintext.as_f64().unwrap();

            assert!(
                (decrypted - expected).abs() < 0.0001,
                "Failed to decrypt legacy float8. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_bool() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "bool" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_bool", &fixture.ciphertext).await;

            let decrypted: bool = decrypt_via_proxy(id, "encrypted_bool").await;
            let expected = fixture.plaintext.as_bool().unwrap();

            assert_eq!(
                decrypted, expected,
                "Failed to decrypt legacy bool. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    #[tokio::test]
    async fn regression_decrypt_legacy_jsonb() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "jsonb" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_jsonb", &fixture.ciphertext).await;

            let decrypted: Value = decrypt_via_proxy(id, "encrypted_jsonb").await;

            assert_eq!(
                decrypted, fixture.plaintext,
                "Failed to decrypt legacy jsonb. Fixture from commit: {}",
                fixture_set.git_commit
            );
        }
    }

    /// Test JSONB field access (-> operator) on legacy encrypted data
    #[tokio::test]
    async fn regression_jsonb_field_access() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "jsonb" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_jsonb", &fixture.ciphertext).await;

            // Test field access via proxy
            let proxy_client = connect_with_tls(PROXY).await;

            // Access string field
            let sql = "SELECT encrypted_jsonb->'string' FROM encrypted WHERE id = $1";
            let rows = proxy_client.query(sql, &[&id]).await.unwrap();
            let decrypted: Value = rows[0].get(0);
            assert_eq!(
                decrypted, fixture.plaintext["string"],
                "Failed to access 'string' field on legacy jsonb"
            );

            // Access number field
            let sql = "SELECT encrypted_jsonb->'number' FROM encrypted WHERE id = $1";
            let rows = proxy_client.query(sql, &[&id]).await.unwrap();
            let decrypted: Value = rows[0].get(0);
            assert_eq!(
                decrypted, fixture.plaintext["number"],
                "Failed to access 'number' field on legacy jsonb"
            );

            // Access nested field
            let sql = "SELECT encrypted_jsonb->'nested' FROM encrypted WHERE id = $1";
            let rows = proxy_client.query(sql, &[&id]).await.unwrap();
            let decrypted: Value = rows[0].get(0);
            assert_eq!(
                decrypted, fixture.plaintext["nested"],
                "Failed to access 'nested' field on legacy jsonb"
            );
        }
    }

    /// Test JSONB array operations on legacy encrypted data
    #[tokio::test]
    async fn regression_jsonb_array_operations() {
        trace();

        let Some(fixture_set) = load_fixtures() else {
            return;
        };

        clear().await;

        for fixture in &fixture_set.fixtures {
            if fixture.data_type != "jsonb" {
                continue;
            }

            let id = random_id();
            insert_encrypted_directly(id, "encrypted_jsonb", &fixture.ciphertext).await;

            let proxy_client = connect_with_tls(PROXY).await;

            // Access array field
            let sql = "SELECT encrypted_jsonb->'array_number' FROM encrypted WHERE id = $1";
            let rows = proxy_client.query(sql, &[&id]).await.unwrap();
            let decrypted: Value = rows[0].get(0);
            assert_eq!(
                decrypted, fixture.plaintext["array_number"],
                "Failed to access 'array_number' field on legacy jsonb"
            );
        }
    }
}
