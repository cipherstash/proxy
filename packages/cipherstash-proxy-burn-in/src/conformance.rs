//! Deterministic protocol and encrypted-data conformance checks.
//!
//! Setup first proves representative fixture values are ciphertext at rest.
//! The checks below then read those values through Proxy and cover typed
//! decryption, transactional encrypted CRUD, rollback, SQL error recovery, and
//! concurrent connections.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::time::timeout;

use crate::database::{self, DatabaseTarget};

pub async fn run(
    proxy_database: &DatabaseTarget,
    direct_database: &DatabaseTarget,
    eql_path: &Path,
) -> Result<()> {
    let _run_lock = database::acquire_run_lock(direct_database).await?;
    database::ensure_eql_installed(direct_database, eql_path).await?;
    timeout(
        database::MIGRATION_TIMEOUT,
        database::migrate(proxy_database, direct_database),
    )
    .await
    .context("fixture migration timed out")??;
    let mut client = database::connect(proxy_database).await?;

    client
        .simple_query("SELECT current_database(), current_user")
        .await
        .context("simple-query startup conformance")?;

    let sample = client
        .query_one(
            "SELECT scalar, nullable_text, binary_value, tags, document, wide_text \
             FROM burnin_type_lab_samples WHERE id = $1",
            &[&1_i32],
        )
        .await
        .context("extended-query type conformance")?;
    anyhow::ensure!(sample.get::<_, i32>(0) == 10, "scalar value was corrupted");
    anyhow::ensure!(
        sample.get::<_, Option<String>>(1).is_none(),
        "NULL was corrupted"
    );
    anyhow::ensure!(
        sample.get::<_, Vec<u8>>(2) == [0, 1, 2, 255],
        "bytea was corrupted"
    );
    anyhow::ensure!(
        sample.get::<_, Vec<String>>(3) == ["alpha", "one"],
        "array was corrupted"
    );
    anyhow::ensure!(
        sample.get::<_, Value>(4)["kind"] == "alpha",
        "jsonb was corrupted"
    );
    anyhow::ensure!(
        sample.get::<_, String>(5) == "wide-alpha-".repeat(40),
        "wide text was corrupted"
    );

    let transaction = client
        .transaction()
        .await
        .context("starting CRUD transaction")?;
    let fixture_id = 900_001_i32;
    transaction
        .execute(
            "INSERT INTO burnin_commerce_customers (id, name) VALUES ($1, $2)",
            &[&fixture_id, &"conformance-customer"],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce_products (id, sku, price_cents) VALUES ($1, $2, $3)",
            &[&fixture_id, &"CONF-900001", &2_499_i32],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce_orders (id, customer_id, status) VALUES ($1, $2, $3)",
            &[&fixture_id, &fixture_id, &"open"],
        )
        .await?;
    transaction.execute(
        "INSERT INTO burnin_commerce_order_lines (order_id, line_number, product_id, quantity) VALUES ($1, 1, $2, 2)",
        &[&fixture_id, &fixture_id],
    ).await?;
    let total: Option<i64> = transaction
        .query_one(
            "SELECT sum(p.price_cents * l.quantity)::bigint \
         FROM burnin_commerce_orders o \
         JOIN burnin_commerce_order_lines l ON l.order_id = o.id \
         JOIN burnin_commerce_products p ON p.id = l.product_id \
         WHERE o.id = $1",
            &[&fixture_id],
        )
        .await?
        .try_get(0)
        .context("decoding joined CRUD total")?;
    anyhow::ensure!(total == Some(4_998), "joined CRUD result was corrupted");
    transaction
        .execute(
            "UPDATE burnin_commerce_orders SET status = 'paid' WHERE id = $1",
            &[&fixture_id],
        )
        .await?;
    transaction
        .rollback()
        .await
        .context("rolling back CRUD transaction")?;
    let rolled_back: i64 = client
        .query_one(
            "SELECT count(*) FROM burnin_commerce_orders WHERE id = $1",
            &[&fixture_id],
        )
        .await?
        .get(0);
    anyhow::ensure!(rolled_back == 0, "transaction rollback leaked a row");

    let error = client
        .execute(
            "INSERT INTO burnin_commerce_products (id, sku, price_cents) VALUES ($1, $2, $3)",
            &[&900_002_i32, &"INVALID-PRICE", &0_i32],
        )
        .await
        .expect_err("check constraint should reject a zero price");
    anyhow::ensure!(
        error.code().is_some_and(|code| code.code() == "23514"),
        "unexpected SQLSTATE: {error}"
    );
    let recovered: i32 = client
        .query_one("SELECT $1::integer", &[&42_i32])
        .await?
        .get(0);
    anyhow::ensure!(
        recovered == 42,
        "connection did not recover after SQL error"
    );

    let mut tasks = Vec::new();
    for worker in 0..8_i32 {
        let target = proxy_database.clone();
        tasks.push(tokio::spawn(async move {
            let client = database::connect(&target).await?;
            for iteration in 0..25_i32 {
                let value: i32 = client
                    .query_one("SELECT $1::integer + $2::integer", &[&worker, &iteration])
                    .await?
                    .get(0);
                anyhow::ensure!(value == worker + iteration, "concurrent result mismatch");
            }
            Result::<()>::Ok(())
        }));
    }
    for task in tasks {
        task.await.context("conformance worker panicked")??;
    }

    println!(
        "conformance passed: simple, extended, types, CRUD, rollback, error recovery, concurrency"
    );
    Ok(())
}
