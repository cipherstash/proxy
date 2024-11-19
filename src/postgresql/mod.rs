mod backend;
mod bind;
mod format_code;
mod frontend;
mod parse;
mod protocol;
mod query;

use crate::error::{Error, ProtocolError};
use crate::{SIZE_I32, SIZE_U8};

use bytes::{BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Cursor};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

pub use backend::Backend;
pub use bind::{Bind, BindParam};
pub use format_code::FormatCode;
pub use frontend::Frontend;

// 1 minute
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1);

// Used in the StartupMessage to indicate regular handshake.
const PROTOCOL_VERSION_NUMBER: i32 = 196608;

/// Protocol message codes.
const BIND: u8 = b'B';
const PARSE: u8 = b'P';
const QUERY: u8 = b'Q';
const NULL: i32 = -1;

pub trait BytesMutReadString {
    fn read_string(&mut self) -> Result<String, Error>;
}

impl BytesMutReadString for Cursor<&BytesMut> {
    /// Should only be used when reading strings from the message protocol.
    /// Can be used to read multiple strings from the same message which are separated by the null byte
    fn read_string(&mut self) -> Result<String, Error> {
        let mut buf = Vec::with_capacity(512);
        match self.read_until(b'\0', &mut buf) {
            Ok(_) => Ok(String::from_utf8_lossy(&buf[..buf.len() - 1]).to_string()),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    code: u8,
    bytes: BytesMut,
}

async fn read_message<C: AsyncRead + Unpin>(mut client: C) -> Result<Message, Error> {
    let code = client.read_u8().await?;
    let len = client.read_i32().await?;

    // debug!("[read_message]");
    // debug!("code: {}", code as char);
    // debug!("len: {len}");

    // Detect unexpected message len and avoid panic on read_exact
    // Len must be at least 4 bytes (4 bytes for len/i32)
    if (len as usize) < SIZE_I32 {
        error!(code = code, len = len, "Unexpected message length");
        return Err(ProtocolError::UnexpectedMessageLength {
            code,
            len: len as usize,
        }
        .into());
    }

    let capacity = len as usize + SIZE_U8; //len plus len of code
    let mut bytes = BytesMut::with_capacity(capacity);

    bytes.put_u8(code);
    bytes.put_i32(len);

    let slice_start = bytes.len();

    // Capacity and len are not the same!!
    // resize populates the buffer with 0s
    bytes.resize(capacity, b'0');

    client.read_exact(&mut bytes[slice_start..]).await?;

    let message = Message { code, bytes };

    Ok(message)
}
