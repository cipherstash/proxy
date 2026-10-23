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
    use tokio_postgres::error::SqlState;
    use uuid::Uuid;

    const INVALID_INBOUND_PAYLOAD: &str = "Invalid encrypted value. For help visit \
        https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-invalid-inbound-eql-payload";

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

    async fn encrypt_json(
        table: &str,
        column: &str,
        plaintext: serde_json::Value,
    ) -> serde_json::Value {
        let prepared = PreparedPlaintext::new(
            Cow::Owned(json_search_config(table, column)),
            Identifier::new(table, column),
            Plaintext::Json(Some(plaintext)),
            EqlOperation::Store,
        );
        let mut outputs =
            encrypt_eql_v3(cipher().await, vec![prepared], &EqlEncryptOpts::default())
                .await
                .expect("application-side JSON encryption must succeed");
        let EqlOutputV3::Store(ciphertext) = outputs.remove(0) else {
            panic!("JSON encryption must return a stored payload");
        };
        serde_json::to_value(ciphertext).unwrap()
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

    async fn query_json_selector(table: &str, column: &str, path: &str) -> String {
        let config = json_search_config(table, column);
        let index_type = config.indexes[0].index_type.clone();
        let prepared = PreparedPlaintext::new(
            Cow::Owned(config),
            Identifier::new(table, column),
            Plaintext::from(path),
            EqlOperation::Query(&index_type, QueryOp::SteVecSelector),
        );
        let mut outputs =
            encrypt_eql_v3(cipher().await, vec![prepared], &EqlEncryptOpts::default())
                .await
                .expect("application-side selector encryption must succeed");
        let EqlOutputV3::Query(query) = outputs.remove(0) else {
            panic!("selector encryption must return a query-only payload");
        };
        let serde_json::Value::String(selector) = serde_json::to_value(query).unwrap() else {
            panic!("selector encryption must return a bare selector hash");
        };
        selector
    }

    #[tokio::test]
    async fn accepts_pre_encrypted_parameter_with_the_configured_default_keyset() {
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
    async fn accepts_pre_encrypted_ste_vec_parameter_for_storage_and_readback() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = serde_json::json!({
            "patient": { "name": "Ada Lovelace" },
            "allergies": ["pollen", "latex"]
        });
        let payload = encrypt_json("encrypted", "encrypted_jsonb", plaintext.clone()).await;

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .unwrap();

        let row = client
            .query_one(
                "SELECT encrypted_jsonb FROM encrypted WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, serde_json::Value>(0), plaintext);
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
            INVALID_INBOUND_PAYLOAD
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
    async fn accepts_bare_selector_hashes_as_parameters_and_literals() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let plaintext = serde_json::json!({
            "patient": { "name": "Ada Lovelace" }
        });

        client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &plaintext],
            )
            .await
            .unwrap();

        let selector = query_json_selector("encrypted", "encrypted_jsonb", "$.patient.name").await;
        let row = client
            .query_one(
                "SELECT encrypted_jsonb -> $1 FROM encrypted WHERE id = $2",
                &[&selector, &id],
            )
            .await
            .unwrap();
        assert_eq!(
            row.get::<_, serde_json::Value>(0),
            serde_json::json!("Ada Lovelace")
        );

        let row = client
            .simple_query(&format!(
                "SELECT jsonb_path_query_first(encrypted_jsonb, '{selector}') \
                 FROM encrypted WHERE id = '{id}'"
            ))
            .await
            .unwrap()
            .into_iter()
            .find_map(|message| match message {
                tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                _ => None,
            })
            .expect("bare selector literal must return an extracted value");
        assert_eq!(row, "\"Ada Lovelace\"");
    }

    #[tokio::test]
    async fn rejects_a_ste_vec_payload_with_tampered_non_root_ciphertext() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let mut payload = encrypt_json(
            "encrypted",
            "encrypted_jsonb",
            serde_json::json!({
                "patient": { "name": "Ada Lovelace", "active": true }
            }),
        )
        .await;

        let entries = payload["sv"].as_array_mut().unwrap();
        assert!(entries.len() > 1);
        let mut ciphertext = entries[1]["c"]
            .as_str()
            .unwrap()
            .chars()
            .collect::<Vec<_>>();
        let different = ciphertext
            .iter()
            .position(|candidate| *candidate != ciphertext[0])
            .unwrap();
        ciphertext.swap(0, different);
        entries[1]["c"] = ciphertext.into_iter().collect::<String>().into();

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .expect_err("tampered non-root ciphertext must be rejected");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );
    }

    #[tokio::test]
    async fn rejects_a_ste_vec_payload_with_a_tampered_array_marker() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let mut payload = encrypt_json(
            "encrypted",
            "encrypted_jsonb",
            serde_json::json!({ "allergies": ["pollen", "latex"] }),
        )
        .await;
        let entry = payload["sv"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry.get("a").is_some())
            .expect("an array value must produce an array-marked SteVec entry");
        entry["a"] = false.into();

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &payload],
            )
            .await
            .expect_err("tampered SteVec array metadata must be rejected");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );
    }

    #[tokio::test]
    async fn rejects_ste_vec_terms_spliced_from_another_plaintext() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let x = encrypt_json(
            "encrypted",
            "encrypted_jsonb",
            serde_json::json!({ "patient": { "name": "indexed as x" } }),
        )
        .await;
        let mut y = encrypt_json(
            "encrypted",
            "encrypted_jsonb",
            serde_json::json!({ "patient": { "name": "decrypts as y" } }),
        )
        .await;
        let x_term = x["sv"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|entry| entry.get("op").cloned())
            .expect("ordered string entries must carry an ordering term");
        let y_entry = y["sv"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry.get("op").is_some())
            .expect("ordered string entries must carry an ordering term");
        y_entry["op"] = x_term;

        let error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &y],
            )
            .await
            .expect_err("spliced SteVec SEM terms must be rejected");
        assert_eq!(
            error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );
    }

    #[tokio::test]
    async fn rejects_mismatched_keyset_metadata_for_scalar_and_ste_vec_payloads() {
        let client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;

        let mut scalar: EqlCiphertextV3 = serde_json::from_str(
            &encrypt_text(
                "encrypted",
                "encrypted_text",
                "wrong scalar keyset metadata",
            )
            .await,
        )
        .unwrap();
        let EqlCiphertextV3::Encrypted(scalar) = &mut scalar else {
            unreachable!()
        };
        scalar.ciphertext.keyset_id = Some(Uuid::new_v4());
        let scalar = serde_json::to_string(&scalar).unwrap();

        let scalar_error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&random_id(), &scalar],
            )
            .await
            .expect_err("false scalar keyset metadata must be rejected");
        assert_eq!(
            scalar_error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );

        let mut ste_vec: EqlCiphertextV3 = serde_json::from_value(
            encrypt_json(
                "encrypted",
                "encrypted_jsonb",
                serde_json::json!({ "patient": { "name": "wrong SteVec keyset metadata" } }),
            )
            .await,
        )
        .unwrap();
        let EqlCiphertextV3::SteVec(payload) = &mut ste_vec else {
            unreachable!()
        };
        payload.key_header.keyset_id = Some(Uuid::new_v4());
        let ste_vec = serde_json::to_value(&ste_vec).unwrap();

        let ste_vec_error = client
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&random_id(), &ste_vec],
            )
            .await
            .expect_err("false SteVec keyset metadata must be rejected");
        assert_eq!(
            ste_vec_error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );
    }

    #[tokio::test]
    async fn invalid_payload_aborts_only_the_current_transaction() {
        let mut client = connect_with_tls(*PROXY).await;
        clear_with_client(&client).await;
        let id = random_id();
        let malformed = serde_json::json!({
            "v": 3,
            "i": { "t": "encrypted", "c": "encrypted_jsonb" },
            "c": "not a ciphertext"
        });
        let transaction = client.transaction().await.unwrap();

        let error = transaction
            .execute(
                "INSERT INTO encrypted (id, encrypted_jsonb) VALUES ($1, $2)",
                &[&id, &malformed],
            )
            .await
            .expect_err("invalid payload must abort the statement");
        assert_eq!(error.code(), Some(&SqlState::INVALID_TEXT_REPRESENTATION));
        assert_eq!(
            error.as_db_error().unwrap().message(),
            INVALID_INBOUND_PAYLOAD
        );

        let aborted = transaction
            .query_one("SELECT 1", &[])
            .await
            .expect_err("transaction must remain aborted until rollback");
        assert_eq!(aborted.code(), Some(&SqlState::IN_FAILED_SQL_TRANSACTION));
        transaction.rollback().await.unwrap();

        let row = client.query_one("SELECT 1", &[]).await.unwrap();
        assert_eq!(row.get::<_, i32>(0), 1);
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
            INVALID_INBOUND_PAYLOAD
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
            INVALID_INBOUND_PAYLOAD
        );
    }
}
