//! Proxy's schema loader, run directly against PostgreSQL.

#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, get_database_port, trace};
    use cipherstash_proxy::{proxy::SchemaManager, DatabaseConfig};

    const ROLE: &str = "schema_loader_narrow_path";

    /// The loader reads only the schemas on its own search path. An encrypted
    /// column outside them must still count, or Proxy forwards a write to a
    /// table it cannot find as if no column were encrypted.
    #[tokio::test]
    async fn encrypted_columns_off_the_loaders_search_path_are_counted() {
        trace();

        let postgres = connect_with_tls(get_database_port()).await;
        postgres
            .batch_execute(&format!(
                "
                DROP SCHEMA IF EXISTS schema_loader_visible CASCADE;
                DROP SCHEMA IF EXISTS schema_loader_hidden CASCADE;
                DROP ROLE IF EXISTS {ROLE};
                CREATE SCHEMA schema_loader_visible;
                CREATE SCHEMA schema_loader_hidden;
                CREATE TABLE schema_loader_visible.notes (id bigint, body text);
                CREATE TABLE schema_loader_hidden.secrets (id bigint, secret eql_v3_text_search);
                CREATE ROLE {ROLE} LOGIN PASSWORD 'p@ssword';
                ALTER ROLE {ROLE} SET search_path = schema_loader_visible;
                GRANT USAGE ON SCHEMA schema_loader_visible, schema_loader_hidden TO {ROLE};
                GRANT SELECT ON ALL TABLES IN SCHEMA schema_loader_visible TO {ROLE};
                GRANT SELECT ON ALL TABLES IN SCHEMA schema_loader_hidden TO {ROLE};
                "
            ))
            .await
            .unwrap();

        let config: DatabaseConfig = serde_json::from_value(serde_json::json!({
            "host": "localhost",
            "port": get_database_port(),
            "name": "cipherstash",
            "username": ROLE,
            "password": "p@ssword",
        }))
        .unwrap();
        let snapshot = SchemaManager::init(&config).await.unwrap().load();
        let has_encrypted_columns =
            !snapshot.encrypt_config().is_empty() || snapshot.schema().has_encrypted_columns();

        postgres
            .batch_execute(&format!(
                "
                DROP SCHEMA schema_loader_visible CASCADE;
                DROP SCHEMA schema_loader_hidden CASCADE;
                DROP ROLE {ROLE};
                "
            ))
            .await
            .unwrap();

        assert!(has_encrypted_columns);
    }
}
