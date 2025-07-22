#[cfg(test)]
mod tests {
    use crate::common::{clear, insert_jsonb, query_by, simple_query, trace};
    use crate::support::assert::assert_expected;
    use crate::support::json_path::JsonPath;

    async fn select_jsonb(selector: &str, expected: i32) {
        let selector = JsonPath::new(selector);

        let sql = "SELECT jsonb_array_length(jsonb_path_query(encrypted_jsonb, $1)) FROM encrypted";
        let actual = query_by::<i32>(sql, &selector).await;

        if expected == 0 {
            assert!(actual.is_empty());
        } else {
            let expected = vec![expected];
            assert_expected(&expected, &actual);
        }

        let sql = format!("SELECT jsonb_array_length(jsonb_path_query(encrypted_jsonb, '{selector}')) FROM encrypted");
        let actual = simple_query::<i32>(&sql).await;

        if expected == 0 {
            assert!(actual.is_empty());
        } else {
            let expected = vec![expected];
            assert_expected(&expected, &actual);
        }
    }

    #[tokio::test]
    async fn select_jsonb_string_array_length() {
        trace();

        clear().await;
        insert_jsonb().await;

        select_jsonb("$.array_string[@]", 2).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_length_number() {
        trace();

        clear().await;

        insert_jsonb().await;

        select_jsonb("$.array_number[@]", 2).await;
    }

    #[tokio::test]
    async fn select_jsonb_array_length_unknown() {
        trace();

        clear().await;
        insert_jsonb().await;

        select_jsonb("$.blah", 0).await;
    }
}
