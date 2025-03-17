fn protected_string_deserializer<'de, D>(deserializer: D) -> Result<Protected<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Protected::new(s))
}
