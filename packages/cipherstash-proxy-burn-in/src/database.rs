//! Database lifecycle for encrypted burn-in fixtures.
//!
//! EQL itself is installed directly because Proxy cannot map statements until
//! its domains exist. Fixture DDL and seed writes then go through Proxy so DDL
//! triggers schema/encrypt-config reloads and seed values are encrypted. DDL
//! and seed use different Proxy connections because each connection snapshots
//! those configurations when it opens.

use std::{fmt, path::Path, str::FromStr};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio_postgres::{config::Host, Client, NoTls};

use crate::{SCHEMA_MIGRATION, SEED_MIGRATION};

const RUN_LOCK_ID: i64 = 0x4353_4255_524e_494e;

#[derive(Clone)]
pub struct DatabaseTarget {
    config: tokio_postgres::Config,
    identity: String,
}

impl DatabaseTarget {
    pub fn hostname(&self) -> Result<&str> {
        match self.config.get_hosts() {
            [Host::Tcp(host)] => Ok(host),
            _ => anyhow::bail!("burn-in requires exactly one TCP database host"),
        }
    }

    pub fn port(&self) -> Result<u16> {
        match self.config.get_ports() {
            [] => Ok(5432),
            [port] => Ok(*port),
            _ => anyhow::bail!("burn-in requires exactly one database port"),
        }
    }

    pub fn configure_proxy_upstream(&self, command: &mut tokio::process::Command) -> Result<()> {
        command
            .env("CS_DATABASE__HOST", self.hostname()?)
            .env("CS_DATABASE__PORT", self.port()?.to_string())
            .env(
                "CS_DATABASE__NAME",
                self.config.get_dbname().unwrap_or("postgres"),
            )
            .env(
                "CS_DATABASE__USERNAME",
                self.config.get_user().unwrap_or("postgres"),
            );
        if let Some(password) = self.config.get_password() {
            command.env(
                "CS_DATABASE__PASSWORD",
                std::str::from_utf8(password).context("database password is not UTF-8")?,
            );
        } else {
            command.env_remove("CS_DATABASE__PASSWORD");
        }
        Ok(())
    }
}

impl FromStr for DatabaseTarget {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let config = value
            .parse::<tokio_postgres::Config>()
            .map_err(|_| "invalid PostgreSQL connection configuration".to_string())?;
        let host = match config.get_hosts() {
            [Host::Tcp(host)] => host.clone(),
            [Host::Unix(path)] => path.to_str().unwrap_or("unix-socket").to_string(),
            _ => "multiple-hosts".to_string(),
        };
        let port = config.get_ports().first().copied().unwrap_or(5432);
        let user = config.get_user().unwrap_or("postgres").to_string();
        let database = config.get_dbname().unwrap_or("postgres").to_string();
        Ok(Self {
            config,
            identity: format!("postgresql://{user}@{host}:{port}/{database}"),
        })
    }
}

impl fmt::Debug for DatabaseTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DatabaseTarget")
            .field(&self.identity)
            .finish()
    }
}

impl fmt::Display for DatabaseTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.identity)
    }
}

pub async fn connect(target: &DatabaseTarget) -> Result<Client> {
    let (client, connection) = target
        .config
        .connect(NoTls)
        .await
        .with_context(|| format!("connecting to {target}"))?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("database connection failed: {error}");
        }
    });
    Ok(client)
}

pub async fn acquire_run_lock(target: &DatabaseTarget) -> Result<Client> {
    let client = connect(target).await?;
    let acquired: bool = client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&RUN_LOCK_ID])
        .await
        .context("acquiring the burn-in database lock")?
        .get(0);
    anyhow::ensure!(
        acquired,
        "another burn-in or conformance run already owns the database fixtures"
    );
    Ok(client)
}

/// Ensure the EQL domains exist before fixture DDL is sent through Proxy.
pub async fn ensure_eql_installed(direct_database: &DatabaseTarget, eql_path: &Path) -> Result<()> {
    let client = connect(direct_database).await?;
    let installed: bool = client
        .query_one(
            "SELECT EXISTS (\
             SELECT 1 FROM pg_type t \
             JOIN pg_namespace n ON n.oid = t.typnamespace \
             WHERE n.nspname = 'public' AND t.typname = 'eql_v3_text'\
             )",
            &[],
        )
        .await
        .context("checking whether EQL is installed")?
        .get(0);
    if installed {
        return Ok(());
    }

    let eql = tokio::fs::read_to_string(eql_path).await.with_context(|| {
        format!(
            "EQL is not installed and its migration could not be read from {}; run `mise run eql:download` or pass --eql-path",
            eql_path.display()
        )
    })?;
    client
        .batch_execute(&eql)
        .await
        .with_context(|| format!("installing EQL from {}", eql_path.display()))?;
    Ok(())
}

/// Create and seed fixtures through Proxy so DDL reloads its schema and every
/// value assigned to an EQL domain traverses the encryption path.
pub async fn migrate(
    proxy_database: &DatabaseTarget,
    direct_database: &DatabaseTarget,
) -> Result<()> {
    // A connection snapshots Proxy's schema and encrypt config when it opens.
    // Apply DDL on one connection, let Proxy reload, then open a fresh
    // connection whose snapshot includes the new encrypted fixture columns.
    let ddl_client = connect(proxy_database).await?;
    ddl_client
        .batch_execute(SCHEMA_MIGRATION)
        .await
        .context("applying burn-in schema migration through Proxy")?;
    drop(ddl_client);

    let client = connect(proxy_database).await?;
    client
        .batch_execute(SEED_MIGRATION)
        .await
        .context("clearing burn-in fixtures through Proxy")?;

    seed_sample(
        &client,
        1,
        10,
        None,
        vec![0, 1, 2, 255],
        vec!["alpha".into(), "one".into()],
        json!({"kind": "alpha", "enabled": true}),
        "wide-alpha-".repeat(40),
    )
    .await?;
    seed_sample(
        &client,
        2,
        20,
        Some("second".into()),
        vec![0x10, 0x20, 0x30, 0x40],
        vec![],
        json!({"kind": "beta", "count": 2}),
        "wide-beta-".repeat(40),
    )
    .await?;
    seed_sample(
        &client,
        3,
        30,
        None,
        vec![0xde, 0xad, 0xbe, 0xef],
        vec!["nullable".into()],
        json!({"kind": "gamma", "values": [1, 2, 3]}),
        "wide-gamma-".repeat(40),
    )
    .await?;
    seed_sample(
        &client,
        4,
        40,
        Some("fourth".into()),
        vec![0xca, 0xfe, 0xba, 0xbe],
        vec!["delta".into(), "four".into()],
        json!({"kind": "delta", "value": null}),
        "wide-delta-".repeat(40),
    )
    .await?;
    assert_seed_is_encrypted(direct_database).await?;
    Ok(())
}

async fn assert_seed_is_encrypted(direct_database: &DatabaseTarget) -> Result<()> {
    // Query around Proxy and inspect the JSON-backed domains themselves. A
    // successful round trip through Proxy is insufficient proof: an unmappable
    // statement can be passed through and appear correct while storing plaintext.
    let client = connect(direct_database).await?;
    let row = client
        .query_one(
            "SELECT scalar::jsonb, document::jsonb, wide_text::jsonb \
             FROM burnin_type_lab_samples WHERE id = 1",
            &[],
        )
        .await
        .context("reading seeded ciphertext directly from PostgreSQL")?;

    for (index, column) in ["scalar", "document", "wide_text"].into_iter().enumerate() {
        let ciphertext: Value = row.get(index);
        anyhow::ensure!(
            ciphertext.get("c").is_some() && ciphertext.get("v").is_some(),
            "{column} was stored as plaintext instead of EQL ciphertext: {ciphertext}"
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn seed_sample(
    client: &Client,
    id: i32,
    scalar: i32,
    nullable_text: Option<String>,
    binary_value: Vec<u8>,
    tags: Vec<String>,
    document: Value,
    wide_text: String,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO burnin_type_lab_samples \
             (id, scalar, nullable_text, binary_value, tags, document, wide_text) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &id,
                &scalar,
                &nullable_text,
                &binary_value,
                &tags,
                &document,
                &wide_text,
            ],
        )
        .await
        .with_context(|| format!("seeding burn-in sample {id} through Proxy"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_target_redacts_password() {
        let target: DatabaseTarget = "postgresql://alice:super-secret@localhost:5544/app"
            .parse()
            .unwrap();
        assert_eq!(target.to_string(), "postgresql://alice@localhost:5544/app");
        assert!(!format!("{target:?}").contains("super-secret"));
    }
}
