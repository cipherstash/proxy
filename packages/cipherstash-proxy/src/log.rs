use std::sync::Once;
use tracing_subscriber::filter::{Directive, EnvFilter};
use tracing_subscriber::FmtSubscriber;

static INIT: Once = Once::new();

pub const AUTHENTICATION: &str = "authentication";

// Messages related to the various hidden "development mode" messages
pub const DEVELOPMENT: &str = "development";

pub const PROTOCOL: &str = "protocol";

pub fn init() {
    INIT.call_once(|| {
        // TODO: assign level from args
        let log_level: Directive = tracing::Level::DEBUG.into();

        let mut filter = EnvFilter::from_default_env().add_directive(log_level.to_owned());

        let directive = format!("{}={log_level}", DEVELOPMENT).parse().expect("ok");
        filter = filter.add_directive(directive);

        let directive = format!("{}={log_level}", PROTOCOL).parse().expect("ok");
        filter = filter.add_directive(directive);

        let directive = format!("{}={log_level}", AUTHENTICATION)
            .parse()
            .expect("ok");
        filter = filter.add_directive(directive);

        let subscriber = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_file(true)
            .with_line_number(true)
            .with_target(true)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}
