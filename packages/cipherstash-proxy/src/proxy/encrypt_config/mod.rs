/// Immutable encryption-policy snapshots.
mod config;
/// Derives encryption metadata from EQL domain declarations.
pub(crate) mod from_domain;

pub use config::EncryptConfig;
