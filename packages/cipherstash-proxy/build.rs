use std::env;

fn main() {
    if let Ok(eql_version) = env::var("CS_EQL_VERSION") {
        println!(
            "cargo:rustc-env=EQL_VERSION_AT_BUILD_TIME={}",
            eql_version.trim()
        );
    }
}
