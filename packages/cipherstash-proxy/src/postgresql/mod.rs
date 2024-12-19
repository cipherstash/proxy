mod backend;
mod context;
mod format_code;
mod frontend;
mod handler;
mod messages;
mod protocol;
mod startup;

use std::time::Duration;

pub use handler::handler;

pub const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 10);

pub const PROTOCOL_VERSION_NUMBER: i32 = 196608;

pub const SSL_REQUEST: i32 = 80877103;

pub const CANCEL_REQUEST: i32 = 80877102;

pub const SSL_RESPONSE_NO: u8 = b'N';

pub const SSL_RESPONSE_YES: u8 = b'S';
