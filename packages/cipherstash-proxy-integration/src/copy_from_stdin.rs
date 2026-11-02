//! `COPY ... FROM STDIN` through Proxy, which tokio-postgres sends with the extended protocol.
//! Proxy forwards a copy into a table with no encrypted columns, and refuses one into an
//! encrypted column, as it does not encrypt copy data.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use bytes::Bytes;
    use futures_util::SinkExt;
    use std::pin::pin;

    #[tokio::test]
    async fn copy_into_a_plaintext_table() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;

        let (first, second) = (random_id(), random_id());
        let mut sink = pin!(client
            .copy_in("COPY plaintext (id, plaintext) FROM STDIN")
            .await
            .unwrap());
        sink.send(Bytes::from(format!("{first}\talice\n{second}\tbob\n")))
            .await
            .unwrap();
        assert_eq!(sink.as_mut().finish().await.unwrap(), 2);

        let rows = client
            .query(
                "SELECT plaintext FROM plaintext WHERE id = ANY($1) ORDER BY plaintext",
                &[&vec![first, second]],
            )
            .await
            .unwrap();
        let copied: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        assert_eq!(copied, vec!["alice", "bob"]);
    }

    #[tokio::test]
    async fn copy_into_an_encrypted_column_is_refused() {
        trace();
        clear().await;
        let client = connect_with_tls(PROXY).await;

        let err = client
            .copy_in::<_, Bytes>("COPY encrypted (id, encrypted_text) FROM STDIN")
            .await
            .err()
            .unwrap();
        assert!(err.as_db_error().unwrap().message().contains(
            "COPY into encrypted column 'encrypted_text' of table 'encrypted' is not supported"
        ));

        // A COPY with no column list fills every column, including the encrypted ones
        let err = client
            .copy_in::<_, Bytes>("COPY encrypted FROM STDIN")
            .await
            .err()
            .unwrap();
        assert!(err
            .as_db_error()
            .unwrap()
            .message()
            .contains("COPY into encrypted column"));

        // The connection is still usable
        let rows = client.simple_query("SELECT 1").await.unwrap();
        assert!(!rows.is_empty());
    }
}
