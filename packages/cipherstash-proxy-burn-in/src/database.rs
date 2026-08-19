use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio_postgres::{Client, NoTls};

use crate::{SCHEMA_MIGRATION, SEED_MIGRATION};

pub async fn connect(database_url: &str) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .with_context(|| format!("connecting to {database_url}"))?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("database connection failed: {error}");
        }
    });
    Ok(client)
}

/// Ensure the EQL domains exist before fixture DDL is sent through Proxy.
pub async fn ensure_eql_installed(direct_database_url: &str, eql_path: &Path) -> Result<()> {
    let client = connect(direct_database_url).await?;
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
pub async fn migrate(proxy_database_url: &str) -> Result<()> {
    // A connection snapshots Proxy's schema and encrypt config when it opens.
    // Apply DDL on one connection, let Proxy reload, then open a fresh
    // connection whose snapshot includes the new encrypted fixture columns.
    let ddl_client = connect(proxy_database_url).await?;
    ddl_client
        .batch_execute(SCHEMA_MIGRATION)
        .await
        .context("applying burn-in schema migration through Proxy")?;
    drop(ddl_client);

    let client = connect(proxy_database_url).await?;
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

pub async fn wait_until_ready(database_url: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match connect(database_url).await {
            Ok(client) if client.simple_query("SELECT 1").await.is_ok() => return Ok(()),
            _ if tokio::time::Instant::now() >= deadline => {
                anyhow::bail!("proxy did not accept queries at {database_url} within 30 seconds")
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    }
}
