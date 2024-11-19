mod backend;
mod bind;
mod format_code;
mod frontend;
mod parse;
mod protocol;
mod query;

use crate::error::{Error, ProtocolError};
use crate::{SIZE_I32, SIZE_U8};

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

pub use backend::Backend;
pub use bind::{Bind, BindParam};
pub use format_code::FormatCode;
pub use frontend::Frontend;

// 1 minute
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1 * 10);

// Used in the StartupMessage to indicate regular handshake.
const PROTOCOL_VERSION_NUMBER: i32 = 196608;

/// Protocol message codes.
const BIND: u8 = b'B';
const PARSE: u8 = b'P';
const QUERY: u8 = b'Q';
const NULL: i32 = -1;
