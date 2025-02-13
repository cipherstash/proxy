mod from_sql;
mod to_sql;

use crate::log::MAPPER;
use cipherstash_client::encryption::Plaintext;
pub use from_sql::bind_param_from_sql;
use postgres_types::Type;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use tracing::{debug, warn};

pub use from_sql::literal_from_sql;
pub use to_sql::to_sql;
///
/// Fun fact: some clients can specify a parameter type with a parse message
/// The parameter type will overide the underlying column type.
///
/// This means, for example, that Pyscopg can send an i16 for an INT4/i32 or INT8/i64 value.
/// I assume to save some bytes.
///
/// Current flow is to parse the parameter into the Plaintext and then convert to the approprate type
///
///
pub fn to_type(plaintext: Plaintext, postgres_type: &Type) -> Plaintext {
    debug!(target = MAPPER, msg = "Convert plaintext to postgres_type", plaintext = ?plaintext, postgres_type = ?postgres_type );

    match (plaintext, postgres_type) {
        (Plaintext::SmallInt(Some(val)), &Type::INT4) => Plaintext::Int(Some(val as i32)),
        (Plaintext::SmallInt(Some(val)), &Type::INT8) => Plaintext::BigInt(Some(val as i64)),
        (Plaintext::SmallInt(Some(val)), &Type::FLOAT8) => Plaintext::Float(Some(val as f64)),
        (Plaintext::SmallInt(Some(val)), &Type::NUMERIC) => {
            let val = Decimal::from_i16(val);
            Plaintext::Decimal(val)
        }

        (Plaintext::Int(Some(val)), &Type::INT8) => Plaintext::BigInt(Some(val as i64)),
        (Plaintext::Int(Some(val)), &Type::FLOAT8) => Plaintext::Float(Some(val as f64)),
        (Plaintext::Int(Some(val)), &Type::NUMERIC) => {
            let val = Decimal::from_i32(val);
            Plaintext::Decimal(val)
        }

        (Plaintext::BigInt(Some(val)), &Type::NUMERIC) => {
            let val = Decimal::from_i64(val);
            Plaintext::Decimal(val)
        }
        (Plaintext::Float(Some(val)), &Type::NUMERIC) => {
            let val = Decimal::from_f64(val);
            Plaintext::Decimal(val)
        }
        (plaintext, _ty) => {
            warn!(
                msg = "Invalid parameter type conversion",
                postgres_type = ?postgres_type
            );
            plaintext
        }
    }
}
