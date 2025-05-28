/// This module contains test helpers
use temp_env;

/// Runs a function with all CS_ environment variables unset
pub(crate) fn with_no_cs_vars<F: FnOnce() -> R, R>(f: F) -> R {
    let cs_vars = std::env::vars()
        .map(|(k, _v)| k)
        .filter(|k| k.starts_with("CS_"))
        .collect::<Vec<_>>();

    temp_env::with_vars_unset(&cs_vars, f)
}
