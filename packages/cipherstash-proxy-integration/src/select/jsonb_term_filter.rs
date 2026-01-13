//! Tests for term filters on SteVec indexes.
//!
//! The `encrypted_jsonb_filtered` column has a downcase term filter configured,
//! meaning all string values are lowercased before encryption. This enables
//! case-insensitive queries - but note that the decrypted data is also lowercased.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, insert_jsonb_filtered, insert_jsonb_filtered_for_search, query_by_params,
        simple_query, trace,
    };
    use crate::support::json_path::JsonPath;
    use serde_json::Value;

    /// Test case-insensitive equality matching with the downcase term filter.
    /// Data is inserted with mixed case ("Alice", "BOB") but stored/returned as lowercase.
    #[tokio::test]
    async fn select_jsonb_filtered_case_insensitive_eq() {
        trace();
        clear().await;
        insert_jsonb_filtered_for_search().await;

        // Query with lowercase "alice" should match the row originally inserted as "Alice"
        let selector = "name";
        let value = Value::from("alice");

        // Extended protocol
        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE encrypted_jsonb_filtered -> $1 = $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;

        // Term filter lowercases during encryption, so returned value is lowercase
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["name"], "alice");
        assert_eq!(actual[0]["number"], 1);
    }

    /// Test that data inserted with uppercase is stored and returned as lowercase
    #[tokio::test]
    async fn select_jsonb_filtered_uppercase_query_matches() {
        trace();
        clear().await;
        insert_jsonb_filtered_for_search().await;

        // Query with "bob" should match the row originally inserted as "BOB"
        let selector = "name";
        let value = Value::from("bob");

        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE encrypted_jsonb_filtered -> $1 = $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;

        // Both stored and queried values are lowercased
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["name"], "bob");
        assert_eq!(actual[0]["number"], 2);
    }

    /// Test simple protocol with case-insensitive matching
    #[tokio::test]
    async fn select_jsonb_filtered_simple_protocol() {
        trace();
        clear().await;
        insert_jsonb_filtered_for_search().await;

        // Simple protocol query - value is lowercased on both sides
        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE encrypted_jsonb_filtered -> 'name' = '\"charlie\"'";
        let actual = simple_query::<Value>(sql).await;

        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["name"], "charlie");
        assert_eq!(actual[0]["number"], 3);
    }

    /// Test that numbers are not affected by the downcase filter
    #[tokio::test]
    async fn select_jsonb_filtered_numbers_unchanged() {
        trace();
        clear().await;
        insert_jsonb_filtered_for_search().await;

        let selector = "number";
        let value = Value::from(4);

        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE encrypted_jsonb_filtered -> $1 = $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(actual.len(), 1);
        // Name is lowercased by term filter
        assert_eq!(actual[0]["name"], "diana");
        assert_eq!(actual[0]["number"], 4);
    }

    /// Test case-insensitive matching using jsonb_path_query_first
    #[tokio::test]
    async fn select_jsonb_filtered_path_query_case_insensitive() {
        trace();
        clear().await;
        insert_jsonb_filtered_for_search().await;

        let json_path_selector = JsonPath::new("name");
        let value = Value::from("eve");

        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb_filtered, $1) = $2";
        let actual = query_by_params::<Value>(sql, &[&json_path_selector, &value]).await;

        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["name"], "eve");
        assert_eq!(actual[0]["number"], 5);
    }

    /// Test nested field access with term filter
    #[tokio::test]
    async fn select_jsonb_filtered_nested_case_insensitive() {
        trace();
        clear().await;
        let (_id, _) = insert_jsonb_filtered().await;

        // The fixture has nested.title = "Engineer" which gets lowercased
        // Query with lowercase should match
        let json_path_selector = JsonPath::new("nested.title");
        let value = Value::from("engineer");

        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb_filtered, $1) = $2";
        let actual = query_by_params::<Value>(sql, &[&json_path_selector, &value]).await;

        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["nested"]["title"], "engineer");
    }

    /// Test that original fixture data is correctly inserted and queryable
    #[tokio::test]
    async fn select_jsonb_filtered_fixture_data() {
        trace();
        clear().await;
        let (_id, _expected) = insert_jsonb_filtered().await;

        // Query by name field - both query and stored data are lowercased
        let selector = "name";
        let value = Value::from("john");

        let sql =
            "SELECT encrypted_jsonb_filtered FROM encrypted WHERE encrypted_jsonb_filtered -> $1 = $2";
        let actual = query_by_params::<Value>(sql, &[&selector, &value]).await;

        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0]["name"], "john");
        assert_eq!(actual[0]["city"], "melbourne");
    }
}
