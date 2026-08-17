use std::sync::atomic::{AtomicI32, Ordering};

use criterion::{criterion_group, criterion_main, Criterion};
use tokio_postgres::{Client, NoTls};

const SCHEMA: &str = include_str!("../../cipherstash-proxy-burn-in/migrations/0001_schema.sql");
const SEED: &str = include_str!("../../cipherstash-proxy-burn-in/migrations/0002_seed.sql");
static NEXT_ID: AtomicI32 = AtomicI32::new(1_500_000);

fn benchmark_proxy_crud(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("create benchmark runtime");
    let proxy_url = std::env::var("BURN_IN_PROXY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://cipherstash:p%40ssword@localhost:6432/cipherstash".to_owned()
    });
    let direct_url = std::env::var("BURN_IN_DIRECT_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://cipherstash:p%40ssword@localhost:5532/cipherstash".to_owned()
    });
    runtime.block_on(async {
        let direct = connect(&direct_url).await;
        direct
            .batch_execute(SCHEMA)
            .await
            .expect("apply benchmark schema");
        direct
            .batch_execute(SEED)
            .await
            .expect("seed benchmark schema");
    });

    criterion.bench_function("proxy_realistic_crud_transaction", |bencher| {
        bencher.to_async(&runtime).iter(|| {
            let proxy_url = proxy_url.clone();
            async move {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                realistic_crud(&proxy_url, id).await;
            }
        });
    });
}

async fn realistic_crud(proxy_url: &str, id: i32) {
    // A fresh connection per iteration includes the startup/authentication path that a real
    // short-lived application request exercises, followed by one coherent unit of commerce work.
    let mut client = connect(proxy_url).await;
    let transaction = client.transaction().await.expect("start CRUD transaction");
    let name = format!("benchmark-customer-{id}");
    let sku = format!("BENCH-{id}");
    transaction
        .execute(
            "INSERT INTO burnin_commerce.customers (id, name) VALUES ($1, $2)",
            &[&id, &name],
        )
        .await
        .expect("create customer");
    transaction
        .execute(
            "INSERT INTO burnin_commerce.products (id, sku, price_cents) VALUES ($1, $2, $3)",
            &[&id, &sku, &2_499_i32],
        )
        .await
        .expect("create product");
    transaction
        .execute(
            "INSERT INTO burnin_commerce.orders (id, customer_id, status) VALUES ($1, $1, 'open')",
            &[&id],
        )
        .await
        .expect("create order");
    transaction.execute(
        "INSERT INTO burnin_commerce.order_lines (order_id, line_number, product_id, quantity) \
         VALUES ($1, 1, $1, 2), ($1, 2, $1, 1)", &[&id]
    ).await.expect("create order lines");

    let row = transaction
        .query_one(
            "SELECT c.name, count(l.*), sum(p.price_cents::bigint * l.quantity) \
         FROM burnin_commerce.orders o \
         JOIN burnin_commerce.customers c ON c.id = o.customer_id \
         JOIN burnin_commerce.order_lines l ON l.order_id = o.id \
         JOIN burnin_commerce.products p ON p.id = l.product_id \
         WHERE o.id = $1 GROUP BY c.name",
            &[&id],
        )
        .await
        .expect("read order aggregate");
    assert_eq!(row.get::<_, String>(0), name);
    assert_eq!(row.get::<_, i64>(1), 2);
    assert_eq!(row.get::<_, i64>(2), 7_497);

    assert_eq!(
        transaction
            .execute(
                "UPDATE burnin_commerce.orders SET status = 'fulfilled' WHERE id = $1",
                &[&id]
            )
            .await
            .expect("update order"),
        1
    );
    transaction
        .execute(
            "DELETE FROM burnin_commerce.order_lines WHERE order_id = $1",
            &[&id],
        )
        .await
        .expect("delete lines");
    transaction
        .execute("DELETE FROM burnin_commerce.orders WHERE id = $1", &[&id])
        .await
        .expect("delete order");
    transaction
        .execute("DELETE FROM burnin_commerce.products WHERE id = $1", &[&id])
        .await
        .expect("delete product");
    transaction
        .execute(
            "DELETE FROM burnin_commerce.customers WHERE id = $1",
            &[&id],
        )
        .await
        .expect("delete customer");
    transaction.commit().await.expect("commit CRUD transaction");
}

async fn connect(database_url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .unwrap_or_else(|error| panic!("connect to {database_url}: {error}"));
    tokio::spawn(async move {
        connection.await.expect("benchmark database connection");
    });
    client
}

criterion_group!(benches, benchmark_proxy_crud);
criterion_main!(benches);
