use cipherstash_proxy::{
    eql::{Ciphertext, Plaintext},
    log,
};
use common::{connect_with_tls, database_config_with_port, PG_LATEST, PROXY};
use serde_json::Value;
use tracing::info;

mod common;

/*
b"D\0\0\x01\x01\0\x02\0\0\0\x08\0\0\0\0\0\0\0\x01\0\0\0\xeb\x01{\"c\": \"mBbL(37yI+46`6ZLqKPh(mox;C$ADXs4S6lr~#OlkVL?}N1dw~|K^(S2<-(Nux%Ew4d+U9O5PQT#2~$^W}X4baFjie!-cQpY9PaFaL$hHbS~y;-pitWHUp<3Wo=<;Y$Ct\", \"i\": {\"table\": \"users\", \"column\": \"email\"}, \"k\": \"ct\", \"m\": null, \"o\": null, \"u\": null, \"v\": 1}" } PROTOCOL="protocol"

*/
#[tokio::test]
async fn integrate_decrypt() {
    log::init();

    let config = database_config_with_port(PROXY);
    let client = connect_with_tls(&config).await;

    // let sql = "SELECT id, name, email FROM users WHERE id = $1";
    // let rows = client.query(sql, &[&id]).await.expect("ok");

    let id: i64 = 1;
    let email = "hello@cipherstash.com";
    let sql = "SELECT id, name, email FROM users WHERE email = $1";
    // let sql = "SELECT id, name, email FROM users WHERE id = $1 AND email = $2";
    let rows = client.query(sql, &[&id, &email]).await.expect("ok");

    for row in rows {
        info!("{:?}", row);
        // let json_value: Value = row.get("email");
        // let pt: Plaintext = serde_json::from_value(json_value).expect("ok");
        // info!("{:?}", pt);

        // let email: String = row.get("email");
        // info!("{:?}", email);
    }
}
