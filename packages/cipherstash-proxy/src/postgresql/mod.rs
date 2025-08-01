mod backend;
mod context;
mod data;
mod format_code;
mod frontend;
mod handler;
mod mitm;
mod message_buffer;
mod messages;
mod protocol;
mod startup;

pub use context::column::Column;
pub use handler::handler;

pub const PROTOCOL_VERSION_NUMBER: i32 = 196608;

pub const SSL_REQUEST: i32 = 80877103;

pub const CANCEL_REQUEST: i32 = 80877102;

pub const SSL_RESPONSE_NO: u8 = b'N';

pub const SSL_RESPONSE_YES: u8 = b'S';
