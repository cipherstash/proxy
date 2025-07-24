use crate::common::reset_schema_to;

const SCHEMA: &'static str = include_str!("./schema.sql");

pub async fn setup_schema() {
    reset_schema_to(SCHEMA).await
}
