mod group_by;
mod order_by;
mod order_by_with_null;

#[macro_export]
macro_rules! value_for_type {
    (String, $i:expr) => {
        ((b'A' + ($i - 1) as u8) as char).to_string()
    };

    (NaiveDate, $i:expr) => {
        NaiveDate::parse_from_str(&format!("2023-01-{}", $i), "%Y-%m-%d").unwrap()
    };

    ($type:ident, $i:expr) => {
        $i as $type
    };
}
