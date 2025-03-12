#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use cipherstash_proxy::{
        config::{LogFormat, LogLevel},
        Args, Migrate, TandemConfig,
    };
    use fake::{Fake, Faker};

    #[tokio::test]
    async fn migrate_text() {
        trace();
        clear().await;

        let args = Args {
            config_file_path: "".to_string(),
            log_level: LogLevel::Debug,
            log_format: LogFormat::Pretty,
            command: None,
        };

        let config = match TandemConfig::load(&args) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("Configuration Error: {}", err);
                panic!();
            }
        };

        let client = connect_with_tls(PROXY).await;

        for _ in 1..10 {
            let id = id();
            let plaintext = Faker.fake::<String>();

            let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
            client.query(sql, &[&id, &plaintext]).await.unwrap();
        }

        let table = "encrypted".to_string();
        let columns = vec![("plaintext".to_string(), "encrypted_text".to_string())];
        let migrate = Migrate::new(table, columns);

        migrate.run(config).await.unwrap();

        let sql = "SELECT id, plaintext, encrypted_text FROM encrypted";
        let rows = client.query(sql, &[]).await.unwrap();

        for row in rows {
            let pt: String = row.get("plaintext");
            let encrypted: String = row.get("encrypted_text");

            assert_eq!(pt, encrypted);
        }
    }
}
