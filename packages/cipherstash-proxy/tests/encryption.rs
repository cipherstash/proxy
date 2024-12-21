#![allow(dead_code)]

mod common;

use cipherstash_proxy::{
    eql::{Identifier, Plaintext},
    log,
};
use common::{connect_with_tls, database_config_with_port, PROXY};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use tracing::info;

fn generate_random_string(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

fn email() -> String {
    let s = generate_random_string(13);
    format!("{s}@cipherstash.com")
}

#[tokio::test]
async fn integrate_encrypt() {
    log::init();

    // let sql = "INSERT INTO users (email) VALUES ($1)";

    // let identifier = Identifier::new("users", "email");

    // let pt = Plaintext {
    //     plaintext: email(),
    //     identifier,
    //     version: 1,
    //     for_query: None,
    // };
    // let email = serde_json::to_value(pt).unwrap();

    // let config = database_config_with_port(PROXY);
    // let client = connect_with_tls(&config).await;

    // let res = client.query(sql, &[&email]).await;

    // println!("{:?}", res);

    // let sql = "SELECT * FROM users";
    // let result = client.query(sql, &[]).await;

    // println!("{:?}", result);
}
