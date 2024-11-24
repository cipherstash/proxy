mod backend;
mod bind;
mod format_code;
mod frontend;
mod parse;
mod protocol;
mod query;
mod startup;

use crate::error::{Error, ProtocolError};
use crate::{SIZE_I32, SIZE_U8};

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

pub use backend::Backend;
pub use bind::{Bind, BindParam};
pub use format_code::FormatCode;
pub use frontend::Frontend;
pub use protocol::{read_message, read_startup_message, Message, StartupCode, StartupMessage};
pub use startup::{accept_tls, connect_database_with_tls, send_ssl_response};

const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1 * 10);

pub const PROTOCOL_VERSION_NUMBER: i32 = 196608;

pub const SSL_REQUEST: i32 = 80877103;

pub const CANCEL_REQUEST: i32 = 80877102;

/// Protocol message codes.
const BIND: u8 = b'B';
const PARSE: u8 = b'P';
const QUERY: u8 = b'Q';
const NULL: i32 = -1;

const SSL_RESPONSE_YES: u8 = b'S';
const SSL_RESPONSE_NO: u8 = b'N';
