mod bind;
mod format_code;
mod parse;
mod query;

use crate::{Error, ProtocolError, SIZE_I32, SIZE_U8};

pub(crate) use bind::{Bind, BindParam};
pub(crate) use format_code::FormatCode;
// pub(crate) use {Encrypted, Message, PostgreSQL};
// pub(crate) use query::Query;

use bytes::{BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Cursor};
use std::mem;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Encrypted {
    v: usize,
    cfg: usize,
    knd: String,
}

impl Encrypted {
    fn encrypt(&mut self) -> Self {
        Encrypted {
            v: self.v,
            cfg: self.cfg,
            knd: "ct".to_string(),
        }
    }
}

struct Context {
    // parse: Option<Parse>,
    // bind: Option<Bind>,
}

#[derive(Clone, Debug)]
pub struct Message {
    code: u8,
    bytes: BytesMut,
}

pub struct PostgreSQL<C>
where
    C: AsyncRead + Unpin,
{
    client: C,
    startup_complete: bool,
}

// impl<'a> PostgreSQL<'a> {
impl<C> PostgreSQL<C>
where
    C: AsyncRead + Unpin,
{
    pub fn new(client: C) -> Self {
        PostgreSQL {
            client,
            startup_complete: false,
        }
    }

    // rewrite_statement
    // rewrite_parameter
    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        if !self.startup_complete {
            let bytes = self.read_start_up_message().await?;
            return Ok(bytes);
        }

        debug!("[read]");

        let mut message = timeout(CONNECTION_TIMEOUT, self.read_message()).await??;

        match message.code {
            b'Q' => {
                debug!("Query");
                // let query = Query::try_from(&message.bytes.clone())?;
                // debug!("{query:?}");
            }
            b'P' => {
                debug!("Parse");
                // let parse = Parse::try_from(&message.bytes)?;
                // debug!("{parse:?}");
            }
            b'B' => {
                debug!("Bind");
                // let message = message.clone();
                let mut bind = Bind::try_from(&message.bytes)?;
                debug!("{bind:?}");

                for param in bind.param_values.iter_mut() {
                    if let Some(bytes) = intercept_bind_param(param) {
                        param.rewrite(&bytes);
                    }
                    debug!("{param:?}");
                }

                if bind
                    .param_values
                    .iter()
                    .any(|param| param.rewrite_required())
                {
                    debug!("rewrite {bind:?}");
                    let bytes = BytesMut::try_from(bind)?;
                    message.bytes = bytes;
                }
            }
            code => {
                debug!("Code {code}");
            }
        }

        Ok(message.bytes)
    }

    async fn read_message(&mut self) -> Result<Message, Error> {
        let code = self.client.read_u8().await?;
        let len = self.client.read_i32().await?;

        debug!("[read_message]");
        debug!("code: {}", code as char);
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

        self.client.read_exact(&mut bytes[slice_start..]).await?;

        let message = Message { code, bytes };

        Ok(message)
    }

    #[inline]
    async fn read_start_up_message(&mut self) -> Result<BytesMut, Error> {
        let len = self.client.read_i32().await?;
        debug!("[read_start_up_message]");
        debug!("len: {len}");

        let capacity = len as usize;

        let mut bytes = BytesMut::with_capacity(capacity);
        bytes.put_i32(len);
        bytes.resize(capacity, b'0');

        let slice_start = SIZE_I32;
        self.client.read_exact(&mut bytes[slice_start..]).await?;

        // code is the first 4 bytes after len
        let code_bytes: [u8; 4] = [
            bytes.as_ref()[4],
            bytes.as_ref()[5],
            bytes.as_ref()[6],
            bytes.as_ref()[7],
        ];

        let code = i32::from_be_bytes(code_bytes);

        debug!("code: {code}");

        if code == PROTOCOL_VERSION_NUMBER {
            self.startup_complete = true;
        }

        Ok(bytes)
    }
}

fn intercept_bind_param(param: &mut BindParam) -> Option<BytesMut> {
    match param.format_code {
        FormatCode::Text => {
            return encrypt_bind_param(&param.bytes);
        }
        FormatCode::Binary => {
            if param.maybe_jsonb() {
                if let Some(bytes) = encrypt_bind_param(&param.bytes[1..]) {
                    let jsonb = json_to_binary_format(bytes);
                    return Some(jsonb);
                }
            }
        }
    }
    None
}

///
/// Binary jsonb adds a version byte to the front of the encoded json byte string.
///
fn json_to_binary_format(bytes: BytesMut) -> BytesMut {
    let mut jsonb = BytesMut::with_capacity(1 + bytes.len());
    jsonb.put_u8(1);
    jsonb.put_slice(&bytes);
    jsonb
}

fn encrypt_bind_param(bytes: &[u8]) -> Option<BytesMut> {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    debug!("{s:?}");

    match serde_json::from_str::<Encrypted>(&s) {
        Ok(mut pt) => {
            let ct = pt.encrypt();
            debug!("pt {pt:?}");
            debug!("ct {ct:?}");

            let s = serde_json::to_string(&ct).unwrap();
            Some(BytesMut::from(s.as_str()))
        }
        Err(e) => {
            debug!("{e:?}");
            None
        }
    }
}
