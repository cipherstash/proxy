//! ORE Test Fixture Generators
//!
//! These tests generate encrypted ORE fixture data for the EQL project by inserting
//! plaintext values through the CipherStash Proxy (which encrypts them), reading back
//! the raw encrypted JSON directly from PostgreSQL (bypassing the proxy), extracting
//! the ORE block (`ob`) field, and writing SQL fixture files.
//!
//! ## Usage
//!
//! Prerequisites: proxy and PostgreSQL must be running.
//!
//! ```bash
//! # Generate both numeric and text fixtures
//! cargo test -p cipherstash-proxy-integration generate_ore -- --ignored --nocapture
//!
//! # Generate only numeric fixtures
//! cargo test -p cipherstash-proxy-integration generate_ore_numeric -- --ignored --nocapture
//!
//! # Generate only text fixtures
//! cargo test -p cipherstash-proxy-integration generate_ore_text -- --ignored --nocapture
//! ```
//!
//! Output defaults to `/Users/tobyhede/src/encrypt-query-language/tests/` but can be
//! overridden with the `ORE_FIXTURE_OUTPUT_DIR` environment variable.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, get_database_port, PROXY};
    use serde_json::Value;
    use std::fmt::Write as FmtWrite;
    use std::fs;
    use std::path::PathBuf;

    const DEFAULT_OUTPUT_DIR: &str = "/Users/tobyhede/src/encrypt-query-language/tests";

    fn output_dir() -> PathBuf {
        std::env::var("ORE_FIXTURE_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_OUTPUT_DIR))
    }

    /// Determine the ORE field name by inspecting a sample encrypted value.
    /// Returns "ob" or "o" depending on what the proxy produces.
    fn detect_ore_field(json: &Value) -> &'static str {
        if json.get("ob").is_some() {
            "ob"
        } else if json.get("o").is_some() {
            "o"
        } else {
            panic!(
                "Could not detect ORE field in encrypted JSON. \
                 Expected 'ob' or 'o' field. Got: {}",
                json
            );
        }
    }

    /// Unwrap PostgreSQL composite type text representation.
    ///
    /// When casting `eql_v2_encrypted::text`, PostgreSQL returns the value wrapped
    /// in a composite type format like: `("{"c": "...", "ob": [...]}")`.
    /// The inner double-quotes are escaped as `""`.
    fn unwrap_composite_text(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.starts_with("(\"") && trimmed.ends_with("\")") {
            // Strip outer ("...") and unescape doubled double-quotes
            trimmed[2..trimmed.len() - 2].replace("\"\"", "\"")
        } else {
            // Already plain JSON
            trimmed.to_string()
        }
    }

    /// Extract just the ORE block from the encrypted JSON, returning a minimal JSON object.
    fn extract_ore_json(ciphertext: &str) -> String {
        let json_str = unwrap_composite_text(ciphertext);
        let json: Value = serde_json::from_str(&json_str)
            .unwrap_or_else(|e| panic!("Failed to parse encrypted JSON: {e}\nJSON: {json_str}"));

        let field = detect_ore_field(&json);
        let ore_value = &json[field];

        // Return compact JSON with just the ORE field
        format!("{{\"{field}\": {}}}", serde_json::to_string(ore_value).unwrap())
    }

    /// Generate the numeric ORE fixture file with 1000 values (ids 1-1000).
    ///
    /// Inserts integers 1..=1000 as `encrypted_int4` through the proxy, reads back
    /// the encrypted ORE data directly from PostgreSQL, and writes a SQL fixture file.
    #[tokio::test]
    #[ignore]
    async fn generate_ore_numeric_fixture() {
        let proxy_client = connect_with_tls(PROXY).await;
        let db_port = get_database_port();
        let db_client = connect_with_tls(db_port).await;

        // Truncate the encrypted table
        clear().await;

        println!("Inserting 1000 numeric values through proxy...");

        // Insert values 1..=1000 through the proxy
        for i in 1..=1000_i32 {
            let id = i as i64;
            proxy_client
                .execute(
                    "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)",
                    &[&id, &i],
                )
                .await
                .unwrap_or_else(|e| panic!("Failed to insert id={id}: {e}"));

            if i % 100 == 0 {
                println!("  Inserted {i}/1000");
            }
        }

        println!("Reading back encrypted values directly from PostgreSQL...");

        // Read all encrypted values directly (bypassing proxy)
        let rows = db_client
            .query(
                "SELECT id, encrypted_int4::text FROM encrypted WHERE id BETWEEN 1 AND 1000 ORDER BY id",
                &[],
            )
            .await
            .unwrap();

        assert_eq!(
            rows.len(),
            1000,
            "Expected 1000 rows, got {}",
            rows.len()
        );

        // Build the SQL fixture file
        let mut sql = String::new();
        writeln!(sql, "DROP TABLE IF EXISTS ore;").unwrap();
        writeln!(sql, "CREATE TABLE ore").unwrap();
        writeln!(sql, "(").unwrap();
        writeln!(sql, "    id bigint,").unwrap();
        writeln!(sql, "    e eql_v2_encrypted,").unwrap();
        writeln!(sql, "    PRIMARY KEY(id)").unwrap();
        writeln!(sql, ");").unwrap();
        writeln!(sql).unwrap();

        for row in &rows {
            let id: i64 = row.get(0);
            let ciphertext: String = row.get(1);
            let ore_json = extract_ore_json(&ciphertext);

            writeln!(
                sql,
                "INSERT INTO ore(id, e) VALUES ({id}, '{ore_json}'::jsonb::eql_v2_encrypted);"
            )
            .unwrap();
        }

        // Write the file
        let output_path = output_dir().join("ore.sql");
        fs::write(&output_path, &sql)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", output_path.display()));

        println!("Wrote numeric ORE fixture: {}", output_path.display());
        println!("  {} INSERT statements", rows.len());
        println!("  {} total lines", sql.lines().count());
    }

    /// Generate the text ORE fixture file with 100 lexicographically diverse strings.
    ///
    /// Inserts sorted lowercase strings through the proxy, reads back the encrypted
    /// ORE data directly from PostgreSQL, and writes a SQL fixture file that includes
    /// both the plaintext and encrypted values.
    #[tokio::test]
    #[ignore]
    async fn generate_ore_text_fixture() {
        let proxy_client = connect_with_tls(PROXY).await;
        let db_port = get_database_port();
        let db_client = connect_with_tls(db_port).await;

        // Truncate the encrypted table
        clear().await;

        let words = text_fixture_words();
        assert_eq!(words.len(), 100, "Expected exactly 100 words");

        // Verify words are sorted
        let mut sorted = words.clone();
        sorted.sort();
        assert_eq!(words, sorted, "Words must be pre-sorted lexicographically");

        println!("Inserting {} text values through proxy...", words.len());

        // Use IDs starting at 1001 to avoid collisions if run alongside numeric
        let id_offset = 1001_i64;

        for (i, word) in words.iter().enumerate() {
            let id = id_offset + i as i64;
            let text = word.to_string();
            proxy_client
                .execute(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                    &[&id, &text],
                )
                .await
                .unwrap_or_else(|e| panic!("Failed to insert id={id} word='{word}': {e}"));
        }

        println!("Reading back encrypted values directly from PostgreSQL...");

        let max_id = id_offset + words.len() as i64 - 1;
        let rows = db_client
            .query(
                "SELECT id, encrypted_text::text FROM encrypted WHERE id BETWEEN $1 AND $2 ORDER BY id",
                &[&id_offset, &max_id],
            )
            .await
            .unwrap();

        assert_eq!(
            rows.len(),
            words.len(),
            "Expected {} rows, got {}",
            words.len(),
            rows.len()
        );

        // Build the SQL fixture file
        let mut sql = String::new();
        writeln!(sql, "DROP TABLE IF EXISTS ore_text;").unwrap();
        writeln!(sql, "CREATE TABLE ore_text").unwrap();
        writeln!(sql, "(").unwrap();
        writeln!(sql, "    id bigint,").unwrap();
        writeln!(sql, "    plaintext text,").unwrap();
        writeln!(sql, "    e eql_v2_encrypted,").unwrap();
        writeln!(sql, "    PRIMARY KEY(id)").unwrap();
        writeln!(sql, ");").unwrap();
        writeln!(sql).unwrap();

        for (i, row) in rows.iter().enumerate() {
            let _id: i64 = row.get(0);
            let ciphertext: String = row.get(1);
            let ore_json = extract_ore_json(&ciphertext);
            let word = &words[i];

            // Use sequential IDs starting from 1 for the fixture file
            let fixture_id = i + 1;

            // Escape single quotes in the word for SQL safety
            let escaped_word = word.replace('\'', "''");

            writeln!(
                sql,
                "INSERT INTO ore_text(id, plaintext, e) VALUES ({fixture_id}, '{escaped_word}', '{ore_json}'::jsonb::eql_v2_encrypted);"
            )
            .unwrap();
        }

        // Write the file
        let output_path = output_dir().join("ore_text.sql");
        fs::write(&output_path, &sql)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", output_path.display()));

        println!("Wrote text ORE fixture: {}", output_path.display());
        println!("  {} INSERT statements", rows.len());
        println!("  {} total lines", sql.lines().count());
    }

    /// 100 lexicographically sorted, diverse lowercase English words.
    ///
    /// Includes:
    /// - Coverage across A-Z
    /// - Prefix overlaps: "app", "apple", "application"
    /// - Similar starts: "car", "card", "care"
    /// - Varied lengths from 3 to 11 characters
    fn text_fixture_words() -> Vec<&'static str> {
        vec![
            "aardvark",
            "able",
            "abstract",
            "ace",
            "anchor",
            "app",
            "apple",
            "application",
            "arctic",
            "badge",
            "balance",
            "barn",
            "basket",
            "beacon",
            "blaze",
            "bond",
            "brave",
            "breeze",
            "bronze",
            "cabin",
            "canal",
            "car",
            "card",
            "care",
            "castle",
            "cedar",
            "chief",
            "cloud",
            "coral",
            "crane",
            "dagger",
            "dawn",
            "delta",
            "desert",
            "diamond",
            "dome",
            "dragon",
            "drift",
            "eagle",
            "earth",
            "eclipse",
            "ember",
            "epic",
            "falcon",
            "fern",
            "flame",
            "forest",
            "frost",
            "garden",
            "glacier",
            "globe",
            "grain",
            "grove",
            "harbor",
            "hawk",
            "horizon",
            "iron",
            "island",
            "ivory",
            "jade",
            "jasmine",
            "jungle",
            "kernel",
            "kite",
            "knot",
            "lake",
            "lemon",
            "light",
            "lunar",
            "maple",
            "marsh",
            "meadow",
            "mist",
            "noble",
            "north",
            "oak",
            "ocean",
            "olive",
            "orbit",
            "palm",
            "pearl",
            "pine",
            "plaza",
            "quartz",
            "quest",
            "raven",
            "reef",
            "ridge",
            "river",
            "sage",
            "shadow",
            "silver",
            "stone",
            "summit",
            "thunder",
            "valley",
            "willow",
            "wolf",
            "zero",
            "zinc",
        ]
    }
}
