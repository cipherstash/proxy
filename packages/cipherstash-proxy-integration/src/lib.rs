mod common;
mod decrypt;
mod disable_mapping;
mod empty_result;
mod extended_protocol_error_messages;
mod insert;
mod map_concat;
mod map_literals;
mod map_match_index;
mod map_nulls;
mod map_ore_index_order;
mod map_ore_index_where;
mod map_params;
mod map_unique_index;
mod migrate;
mod multitenant;
mod passthrough;
mod pipeline;
mod schema_change;
mod select;
mod simple_protocol;
mod support;
mod update;

#[macro_export]
macro_rules! value_for_type {
    (String, $i:expr) => {
        ((b'A' + ($i - 1) as u8) as char).to_string()
    };

    (NaiveDate, $i:expr) => {
        NaiveDate::parse_from_str(&format!("2023-01-{}", $i), "%Y-%m-%d").unwrap()
    };

    (Value, $i:expr) => {
        serde_json::json!({"n": $i, "s": format!("{}", $i) })
    };

    (bool, $i:expr) => {
        $i % 2 == 0
    };

    ($type:ident, $i:expr) => {
        $i as $type
    };
}
