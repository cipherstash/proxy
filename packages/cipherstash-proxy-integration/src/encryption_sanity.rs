//! Encryption sanity checks - verify data is actually encrypted.
//!
//! These tests insert data through the proxy, then query DIRECTLY from the database
//! (bypassing the proxy) to verify the stored value is encrypted (differs from plaintext).
//!
//! This catches silent mapping failures where data passes through unencrypted.

#[cfg(test)]
mod tests {
    use crate::common::{
        assert_encrypted_jsonb, assert_encrypted_numeric, assert_encrypted_text, clear,
        connect_with_tls, random_id, random_limited, trace, PROXY,
    };
    use chrono::NaiveDate;
    use serde_json;

    #[tokio::test]
    async fn text_encryption_sanity_check() {
        trace();
        clear().await;

        let id = random_id();
        let plaintext = "hello world";

        // Insert through proxy (should encrypt)
        let client = connect_with_tls(PROXY).await;
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext]).await.unwrap();

        // Verify encryption occurred
        assert_encrypted_text(id, "encrypted_text", plaintext).await;

        // Round-trip: query through proxy should decrypt back to original
        let sql = "SELECT encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();
        assert_eq!(rows.len(), 1, "Expected exactly one row for round-trip");
        let decrypted: String = rows[0].get(0);
        assert_eq!(
            decrypted, plaintext,
            "DECRYPTION FAILED: Round-trip value doesn't match original!"
        );
    }

    #[tokio::test]
    async fn jsonb_encryption_sanity_check() {
        trace();
        clear().await;

        let id = random_id();
        let plaintext_json = serde_json::json!({"key": "value", "number": 42});

        // Insert through proxy (should encrypt)
        let client = connect_with_tls(PROXY).await;
        let sql = "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)";
        client.query(sql, &[&id, &plaintext_json]).await.unwrap();

        // Verify encryption occurred
        assert_encrypted_jsonb(id, &plaintext_json).await;

        // Round-trip: query through proxy should decrypt back to original
        let sql = "SELECT encrypted_jsonb FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();
        assert_eq!(rows.len(), 1, "Expected exactly one row for round-trip");
        let decrypted: serde_json::Value = rows[0].get(0);
        assert_eq!(
            decrypted, plaintext_json,
            "DECRYPTION FAILED: Round-trip value doesn't match original!"
        );
    }
}
