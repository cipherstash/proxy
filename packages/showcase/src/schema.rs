use crate::common::{reset_schema_to, PROXY};

const SCHEMA: &str = include_str!("./schema.sql");

pub async fn setup_schema() {
    reset_schema_to(SCHEMA, PROXY).await
}
