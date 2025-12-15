//! Encryption sanity checks - verify data is actually encrypted.
//!
//! These tests insert data through the proxy, then query DIRECTLY from the database
//! (bypassing the proxy) to verify the stored value is encrypted (differs from plaintext).
//!
//! This catches silent mapping failures where data passes through unencrypted.

#[cfg(test)]
mod tests {
    use crate::common::{
        assert_encrypted_jsonb, assert_encrypted_numeric, assert_encrypted_text, clear,
        connect_with_tls, random_id, random_limited, trace, PROXY,
    };
    use chrono::NaiveDate;

    // Tests will be added in subsequent tasks
}
