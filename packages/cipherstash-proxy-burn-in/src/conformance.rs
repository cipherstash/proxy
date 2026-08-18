use anyhow::{Context, Result};
use serde_json::Value;

use crate::database;

pub async fn run(proxy_database_url: &str, direct_database_url: &str) -> Result<()> {
    database::migrate(direct_database_url).await?;
    let mut client = database::connect(proxy_database_url).await?;

    client
        .simple_query("SELECT current_database(), current_user")
        .await
        .context("simple-query startup conformance")?;

    let sample = client
        .query_one(
            "SELECT scalar, nullable_text, binary_value, tags, document, wide_text \
             FROM burnin_type_lab.samples WHERE id = $1",
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
        sample.get::<_, String>(5).len() > 400,
        "wide text was truncated"
    );

    let transaction = client
        .transaction()
        .await
        .context("starting CRUD transaction")?;
    let fixture_id = 900_001_i32;
    transaction
        .execute(
            "INSERT INTO burnin_commerce.customers (id, name) VALUES ($1, $2)",
            &[&fixture_id, &"conformance-customer"],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce.products (id, sku, price_cents) VALUES ($1, $2, $3)",
            &[&fixture_id, &"CONF-900001", &2_499_i32],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO burnin_commerce.orders (id, customer_id, status) VALUES ($1, $2, $3)",
            &[&fixture_id, &fixture_id, &"open"],
        )
        .await?;
    transaction.execute(
        "INSERT INTO burnin_commerce.order_lines (order_id, line_number, product_id, quantity) VALUES ($1, 1, $2, 2)",
        &[&fixture_id, &fixture_id],
    ).await?;
    let total: i64 = transaction
        .query_one(
            "SELECT sum(p.price_cents::bigint * l.quantity) \
         FROM burnin_commerce.orders o \
         JOIN burnin_commerce.order_lines l ON l.order_id = o.id \
         JOIN burnin_commerce.products p ON p.id = l.product_id \
         WHERE o.id = $1",
            &[&fixture_id],
        )
        .await?
        .get(0);
    anyhow::ensure!(total == 4_998, "joined CRUD result was corrupted");
    transaction
        .execute(
            "UPDATE burnin_commerce.orders SET status = 'paid' WHERE id = $1",
            &[&fixture_id],
        )
        .await?;
    transaction
        .rollback()
        .await
        .context("rolling back CRUD transaction")?;
    let rolled_back: i64 = client
        .query_one(
            "SELECT count(*) FROM burnin_commerce.orders WHERE id = $1",
            &[&fixture_id],
        )
        .await?
        .get(0);
    anyhow::ensure!(rolled_back == 0, "transaction rollback leaked a row");

    let error = client
        .execute(
            "INSERT INTO burnin_commerce.products (id, sku, price_cents) VALUES ($1, $2, $3)",
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
        let url = proxy_database_url.to_owned();
        tasks.push(tokio::spawn(async move {
            let client = database::connect(&url).await?;
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
