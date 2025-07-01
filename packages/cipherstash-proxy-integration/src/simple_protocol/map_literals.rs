#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, PROXY};
    use chrono::NaiveDate;
    use tokio_postgres::SimpleQueryMessage::{CommandComplete, Row};

    #[tokio::test]
    async fn simple_protocol_without_encryption() {
        let client = connect_with_tls(PROXY).await;
        let id = random_id();
        let sql = format!("INSERT INTO encrypted (id, plaintext) VALUES ({id}, 'plain')");
        client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        let sql = format!("SELECT id, plaintext FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");
        if let Row(r) = &rows[1] {
            assert_eq!(Some("plain"), r.get(1));
        } else {
            panic!("Unexpected query results: {rows:?}");
        }
    }

    #[tokio::test]
    async fn simple_protocol_text() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        let insert_result = client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {insert_result:?}"),
        }

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");

        if let Row(r) = &rows[1] {
            assert_eq!(Some(id.to_string().as_str()), r.get(0));
            assert_eq!(Some("hello@cipherstash.com"), r.get(1));
        } else {
            panic!("Row(row) expected but got: {:?}", &rows[1]);
        }

        if let CommandComplete(n) = &rows[2] {
            assert_eq!(1, *n);
        } else {
            panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
        }
    }

    #[tokio::test]
    async fn simple_protocol_int2() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_int2: i16 = 42;

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_int2) VALUES ({id}, '{encrypted_int2}')");
        let insert_result = client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {insert_result:?}"),
        }

        let sql = format!("SELECT id, encrypted_int2 FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");

        if let Row(r) = &rows[1] {
            assert_eq!(Some(id.to_string().as_str()), r.get(0));
            assert_eq!(Some(encrypted_int2.to_string().as_str()), r.get(1));
        } else {
            panic!("Row expected but got: {:?}", &rows[1]);
        }

        if let CommandComplete(n) = &rows[2] {
            assert_eq!(1, *n);
        } else {
            panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
        }
    }

    #[tokio::test]
    async fn simple_protocol_date() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_date = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_date) VALUES ({id}, '{encrypted_date}')");

        let insert_result = client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {insert_result:?}"),
        }

        let sql = format!("SELECT id, encrypted_date FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");

        if let Row(r) = &rows[1] {
            assert_eq!(Some(id.to_string().as_str()), r.get(0));
            assert_eq!(Some(encrypted_date.to_string().as_str()), r.get(1));
        } else {
            panic!("Row expected but got: {:?}", &rows[1]);
        }

        if let CommandComplete(n) = &rows[2] {
            assert_eq!(1, *n);
        } else {
            panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
        }
    }

    #[tokio::test]
    async fn simple_protocol_date_with_iso() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_date =
            NaiveDate::parse_from_str("2025-01-01T13:00:00+10:00", "%Y-%m-%dT%H:%M:%S%z").unwrap();

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_date) VALUES ({id}, '{encrypted_date}')");

        let insert_result = client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {insert_result:?}"),
        }

        let sql = format!("SELECT id, encrypted_date FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");

        if let Row(r) = &rows[1] {
            assert_eq!(Some(id.to_string().as_str()), r.get(0));
            assert_eq!(Some(encrypted_date.to_string().as_str()), r.get(1));
        } else {
            panic!("Row expected but got: {:?}", &rows[1]);
        }

        if let CommandComplete(n) = &rows[2] {
            assert_eq!(1, *n);
        } else {
            panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
        }
    }

    #[tokio::test]
    async fn simple_protocol_int4() {
        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_int4: i32 = 42;

        let statements = vec![
            // Unquoted
            format!("INSERT INTO encrypted (id, encrypted_int4) VALUES ({id}, {encrypted_int4})"),
            // Single quoted string
            format!("INSERT INTO encrypted (id, encrypted_int4) VALUES ({id}, '{encrypted_int4}')"),
            // TODO: Unquoted with cast
            // https://linear.app/cipherstash/issue/CIP-1180/handle-explicit-casts-of-encrypted-columns
            // format!("INSERT INTO encrypted (id, encrypted_int4) VALUES ({id}, {encrypted_int4}::smallint)"),
        ];

        for sql in statements {
            clear().await;

            let insert_result = client
                .simple_query(&sql)
                .await
                .expect("INSERT query failed");

            // CmmandComplete does not implement PartialEq, so no equality check with ==
            match &insert_result[0] {
                CommandComplete(n) => assert_eq!(1, *n),
                _unexpected => panic!("unexpected insert result: {insert_result:?}"),
            }

            let sql = format!("SELECT id, encrypted_int4 FROM encrypted WHERE id = {id}");
            let rows = client
                .simple_query(&sql)
                .await
                .expect("SELECT query failed");

            if let Row(r) = &rows[1] {
                assert_eq!(Some(id.to_string().as_str()), r.get(0));
                assert_eq!(Some(encrypted_int4.to_string().as_str()), r.get(1));
            } else {
                panic!("Row expected but got: {:?}", &rows[1]);
            }

            if let CommandComplete(n) = &rows[2] {
                assert_eq!(1, *n);
            } else {
                panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
            }
        }
    }

    #[tokio::test]
    async fn frontend_error_does_not_crash_connection() {
        let client = connect_with_tls(PROXY).await;

        // Statement has the wrong column name
        let sql = format!(
            "INSERT INTO encrypted (id, encrypted) VALUES ({}, 'foo@example.net')",
            random_id()
        );

        let result = client.simple_query(&sql).await;

        assert!(result.is_err());
        let error = result.unwrap_err();

        // The connection should not be closed
        assert!(!error.is_closed());

        // And we can still use the connection
        let sql = format!(
            "INSERT INTO encrypted (id, encrypted_text) VALUES ({}, 'foo@example.net')",
            random_id()
        );
        let result = client.simple_query(&sql).await;

        assert!(result.is_ok());
    }
}
