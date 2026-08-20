//! End-to-end coverage for application-encrypted EQL payloads entering Proxy.

#[cfg(test)]
mod tests {
    use crate::common::{clear_with_client, connect_with_tls, random_id, PROXY};
    use cipherstash_client::{
        encryption::{Plaintext, QueryOp, ScopedCipher},
        eql::{
            encrypt_eql_v3, EqlCiphertextV3, EqlEncryptOpts, EqlOperation, EqlOutputV3, Identifier,
            PreparedPlaintext,
        },
        schema::{column::Index, ColumnConfig, ColumnType},
        zerokms::{ClientKey, ZeroKMSBuilder},
        AutoStrategy, IdentifiedBy,
    };
    use cipherstash_config::column::{ArrayIndexMode, IndexType, SteVecMode};
    use std::{borrow::Cow, sync::Arc};
    use uuid::Uuid;

    async fn cipher() -> Arc<ScopedCipher<AutoStrategy>> {
        let client_id = env("CS_CLIENT_ID", "CS_ENCRYPT__CLIENT_ID")
            .parse()
            .expect("CS_CLIENT_ID must be a UUID");
        let client_key =
            ClientKey::from_hex_v1(client_id, &env("CS_CLIENT_KEY", "CS_ENCRYPT__CLIENT_KEY"))
                .expect("CS_CLIENT_KEY must be valid");
        let zerokms = ZeroKMSBuilder::auto()
            .expect("ZeroKMS credentials must be configured")
            .with_client_key(client_key)
            .build()
            .expect("ZeroKMS client must initialize");
        let keyset_id: Uuid = env("CS_DEFAULT_KEYSET_ID", "CS_ENCRYPT__DEFAULT_KEYSET_ID")
            .parse()
            .expect("CS_DEFAULT_KEYSET_ID must be a UUID");
        Arc::new(
            ScopedCipher::init(Arc::new(zerokms), Some(IdentifiedBy::Uuid(keyset_id)))
                .await
                .expect("scoped cipher must initialize"),
        )
    }

    fn env(primary: &str, nested: &str) -> String {
        std::env::var(primary)
            .or_else(|_| std::env::var(nested))
            .unwrap_or_else(|_| panic!("{primary} must be configured"))
    }

    fn text_search_config(table: &str, column: &str) -> ColumnConfig {
        ColumnConfig::build(format!("{table}/{column}"))
            .casts_as(ColumnType::Text)
            .add_index(Index::new_unique())
            .add_index(Index::new_ope())
            .add_index(Index::new_match())
    }

    fn json_search_config(table: &str, column: &str) -> ColumnConfig {
        ColumnConfig::build(format!("{table}/{column}"))
            .casts_as(ColumnType::Json)
            .add_index(Index::new(IndexType::SteVec {
                prefix: format!("{table}/{column}"),
                term_filters: Vec::new(),
                array_index_mode: ArrayIndexMode::ALL,
                mode: SteVecMode::default(),
            }))
    }

    async fn encrypt_text(table: &str, column: &str, plaintext: &str) -> String {
        let prepared = PreparedPlaintext::new(
            Cow::Owned(text_search_config(table, column)),
            Identifier::new(table, column),
            Plaintext::from(plaintext),
            EqlOperation::Store,
        );
        let mut outputs =
            encrypt_eql_v3(cipher().await, vec![prepared], &EqlEncryptOpts::default())
                .await
                .expect("application-side encryption must succeed");
        let EqlOutputV3::Store(ciphertext) = outputs.remove(0) else {
            panic!("store encryption must return a stored payload");
        };
        serde_json::to_string(&ciphertext).unwrap()
    }

    async fn query_text(table: &str, column: &str, plaintext: &str) -> String {
        let stored: EqlCiphertextV3 =
            serde_json::from_str(&encrypt_text(table, column, plaintext).await).unwrap();
        serde_json::to_string(&stored.into_query_operand()).unwrap()
    }

    async fn query_json(
        table: &str,
        column: &str,
        plaintext: serde_json::Value,
    ) -> serde_json::Value {
        let config = json_search_config(table, column);
        let index_type = config.indexes[0].index_type.clone();
        let prepared = PreparedPlaintext::new(
            Cow::Owned(config),
            Identifier::new(table, column),
            Plaintext::Json(Some(plaintext)),
            EqlOperation::Query(&index_type, QueryOp::Default),
        );
        let mut outputs =
            encrypt_eql_v3(cipher().await, vec![prepared], &EqlEncryptOpts::default())
                .await
                .expect("application-side query encryption must succeed");
        let EqlOutputV3::Query(query) = outputs.remove(0) else {
            panic!("query encryption must return a query-only payload");
        };
        serde_json::to_value(query).unwrap()
    }

    #[tokio::test]
    async fn accepts_pre_encrypted_parameter_for_storage_and_search() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = "encrypted in the application";
        let payload = encrypt_text("encrypted", "encrypted_text", plaintext).await;

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .unwrap();

        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted WHERE encrypted_text = $1",
                &[&payload],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<_, String>(0), plaintext);
    }

    #[tokio::test]
    async fn accepts_pre_encrypted_literal_for_storage() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = "application encrypted literal";
        let payload = encrypt_text("encrypted", "encrypted_text", plaintext).await;
        let payload = payload.replace('\'', "''");

        client
            .simple_query(&format!(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{payload}')"
            ))
            .await
            .unwrap();

        let row = client
            .query_one("SELECT encrypted_text FROM encrypted WHERE id = $1", &[&id])
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), plaintext);
    }

    #[tokio::test]
    async fn accepts_query_only_parameter_for_search() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = "queried with application SEM terms";

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &plaintext],
            )
            .await
            .unwrap();

        let payload = query_text("encrypted", "encrypted_text", plaintext).await;
        let rows = client
            .query(
                "SELECT encrypted_text FROM encrypted WHERE encrypted_text = $1",
                &[&payload],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<_, String>(0), plaintext);
    }

    #[tokio::test]
    async fn accepts_query_only_literal_for_search() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = "queried with literal SEM terms";

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &plaintext],
            )
            .await
            .unwrap();

        let payload = query_text("encrypted", "encrypted_text", plaintext)
            .await
            .replace('\'', "''");
        let rows = client
            .simple_query(&format!(
                "SELECT encrypted_text FROM encrypted WHERE encrypted_text = '{payload}'"
            ))
            .await
            .unwrap();
        let row = rows
            .iter()
            .find_map(|message| match message {
                tokio_postgres::SimpleQueryMessage::Row(row) => Some(row),
                _ => None,
            })
            .expect("query-only literal must match one row");
        assert_eq!(row.get(0), Some(plaintext));
    }

    #[tokio::test]
    async fn rejects_query_only_parameter_for_storage() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let payload = query_text("encrypted", "encrypted_text", "not writable").await;

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .expect_err("query-only payloads must not be accepted for storage");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            "Invalid encrypted value"
        );
    }

    #[tokio::test]
    async fn accepts_query_only_ste_vec_parameter_for_json_search() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = serde_json::json!({
            "patient": { "name": "Ada Lovelace" },
            "active": true
        });

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &plaintext],
            )
            .await
            .unwrap();

        let payload = query_json("encrypted", "encrypted_jsonb", plaintext.clone()).await;
        let rows = client
            .query(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb @> $1",
                &[&payload],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<_, serde_json::Value>(0), plaintext);
    }

    #[tokio::test]
    async fn rejects_payload_for_a_different_destination_with_generic_error() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let payload = encrypt_text("some_other_table", "encrypted_text", "secret").await;
        let mut payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        payload["i"]["t"] = "encrypted".into();
        let payload = payload.to_string();

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .expect_err("destination mismatch must fail closed");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            "Invalid encrypted value"
        );
    }

    #[tokio::test]
    async fn rejects_sem_terms_spliced_from_another_plaintext() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let x: serde_json::Value = serde_json::from_str(
            &encrypt_text("encrypted", "encrypted_text", "indexed as x").await,
        )
        .unwrap();
        let mut y: serde_json::Value = serde_json::from_str(
            &encrypt_text("encrypted", "encrypted_text", "decrypts as y").await,
        )
        .unwrap();
        for term in ["hm", "bf", "ob", "op"] {
            if let Some(value) = x.get(term) {
                y[term] = value.clone();
            }
        }
        let payload = y.to_string();

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .expect_err("spliced SEM terms must fail closed");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            "Invalid encrypted value"
        );
    }
}
