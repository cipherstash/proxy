use bytes::BytesMut;
use postgres_types::{Format, ToSql, Type};
use std::{
    error::Error,
    fmt::{Display, Formatter},
};

#[derive(Debug, Clone)]
pub struct JsonPath(String);

impl JsonPath {
    pub fn new(path: &str) -> Self {
        JsonPath(path.to_string())
    }
}

impl Display for JsonPath {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ToSql for JsonPath {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn Error + Sync + Send>> {
        self.0.to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::JSONPATH
    }

    /// Specify the encode format
    fn encode_format(&self, _ty: &Type) -> Format {
        Format::Text
    }

    postgres_types::to_sql_checked!();
}
