use std::time::Duration;

use my_little_proxy::eql::{Identifier, Plaintext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::sleep;
use tokio_postgres::{Client, NoTls};

const DATABASE_URL: &str = "postgresql://postgres:password@localhost:6432/my_little_proxy";
// const DATABASE_URL: &str = "postgresql://postgres:password@localhost:5432/my_little_proxy";

// CREATE TABLE blah (
//     id bigint GENERATED ALWAYS AS IDENTITY,
//     t TEXT,
//     j JSONB,
//     vtha JSONB,
//     PRIMARY KEY(id)
// );

pub async fn connect() -> Result<Client, tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(DATABASE_URL, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    Ok(client)
}

#[tokio::test]
async fn rewrite_bind_on_insert() {
    let client = connect().await.unwrap();

    // let sql = "INSERT INTO blah (vtha) VALUES ($1)";

    // let identifier = Identifier {
    //     table: "blah".to_string(),
    //     column: "vtha".to_string(),
    // };

    // let pt = Plaintext {
    //     plaintext: "hello".to_string(),
    //     identifier,
    //     version: 1,
    //     for_query: None,
    // };

    // let vtha = serde_json::to_value(pt).unwrap();

    // let res = client.query(sql, &[&vtha]).await;

    // println!("{:?}", res);

    let sql = "SELECT * FROM blah";
    let res = client.query(sql, &[]).await;

    println!("{:?}", res);
}

// #[tokio::test]
// async fn timeout() {
//     let client = connect().await.unwrap();

//     // sleep for 10 seconds using tokio::time::sleep
//     sleep(Duration::from_secs(10)).await;
// }

// #[tokio::test]
// async fn simple_query() {
//     let client = connect().await.unwrap();

//     let set = format!("SELECT 1 WHERE 1 = '{}'", 1);
//     let res = client.simple_query(&set).await;

//     println!("{:?}", res);
// }
