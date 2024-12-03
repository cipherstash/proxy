use cipherstash_proxy::config::DatabaseConfig;

const PORT: u16 = 5532;
const PORT_v17_TLS: u16 = 5517;

pub fn database_config() -> DatabaseConfig {
    DatabaseConfig {
        host: "localhost".to_string(),
        port: PORT,
        database: "mlp".to_string(),
        username: "mlp".to_string(),
        password: "password".to_string(),
        config_reload_interval: 10,
        schema_reload_interval: 10,
        with_tls: false,
    }
}
