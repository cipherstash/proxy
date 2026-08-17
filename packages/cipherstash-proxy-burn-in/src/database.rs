use anyhow::{Context, Result};
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

pub async fn migrate(direct_database_url: &str) -> Result<()> {
    let client = connect(direct_database_url).await?;
    client
        .batch_execute(SCHEMA_MIGRATION)
        .await
        .context("applying burn-in schema migration")?;
    client
        .batch_execute(SEED_MIGRATION)
        .await
        .context("applying burn-in seed migration")?;
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
