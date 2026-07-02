//! EQL v3 variant of the direct-insert cast surface in `eql_regression.rs`.
//!
//! The v2 regression harness reinserts captured ciphertexts with a single
//! `$2::eql_v2_encrypted` cast. v3 has no single encrypted type: direct
//! inserts cast to the column's specific domain (here `eql_v3.text_search`),
//! and the fail-closed domain CHECK validates the envelope on the way in.
//!
//! The full fixture-file regression flow (generate on main, replay on branch)
//! stays v2-only until the mapper speaks v3 and v3 fixtures can be generated.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};

    fn get_database_port() -> u16 {
        std::env::var("CS_DATABASE__PORT")
            .map(|s| s.parse().unwrap())
            .unwrap_or(5617) // Default to TLS port
    }

    ///
    /// Captures a proxy-encrypted v3 ciphertext, reinserts it directly with a
    /// per-domain cast, and decrypts it back through the proxy.
    ///
    #[tokio::test]
    #[ignore = "blocked on eql-mapper v3"]
    async fn insert_v3_ciphertext_directly_and_decrypt_via_proxy() {
        trace();

        clear().await;

        let id = random_id();
        let plaintext = "regression".to_string();

        // Insert via proxy (encrypts to a v3 envelope)
        let proxy_client = connect_with_tls(PROXY).await;
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        proxy_client.query(sql, &[&id, &plaintext]).await.unwrap();

        // Read the raw ciphertext directly from the database (bypassing proxy)
        let db_client = connect_with_tls(get_database_port()).await;
        let sql = "SELECT encrypted_text::text FROM encrypted WHERE id = $1";
        let rows = db_client.query(sql, &[&id]).await.unwrap();
        let ciphertext: String = rows[0].get(0);

        // Reinsert the ciphertext directly, casting to the column's v3 domain
        // (v2 used a single `::eql_v2_encrypted` cast here). The leading
        // `::text` keeps the bound parameter described as text - a bare
        // `$2::eql_v3.text_search` makes Postgres describe the parameter as
        // the jsonb-backed domain, which a text binding cannot satisfy.
        let new_id = random_id();
        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2::text::jsonb::eql_v3.text_search)";
        db_client.query(sql, &[&new_id, &ciphertext]).await.unwrap();

        // Decrypt via proxy
        let sql = "SELECT encrypted_text FROM encrypted WHERE id = $1";
        let rows = proxy_client.query(sql, &[&new_id]).await.unwrap();
        let decrypted: String = rows[0].get(0);

        assert_eq!(plaintext, decrypted);
    }
}
