use crate::error::Error;
use crate::postgresql::protocol::{self};
use crate::postgresql::CONNECTION_TIMEOUT;

use bytes::BytesMut;
use tokio::io::{self, AsyncRead};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Code {
    Authentication,
    BindComplete,
    BackendKeyData,
    CloseComplete,
    CommandComplete,
    CopyBothResponse,
    CopyInResponse,
    CopyOutResponse,
    DataRow,
    EmptyQueryResponse,
    ErrorResponse,
    NoData,
    NoticeResponse,
    NotificationResponse,
    ParameterDescription,
    ParameterStatus,
    ParseComplete,
    PortalSuspended,
    ReadyForQuery,
    RowDescription,
    Unknown(char),
}

pub struct Backend<C>
where
    C: AsyncRead + Unpin,
{
    client: C,
}

impl<C> Backend<C>
where
    C: AsyncRead + Unpin,
{
    pub fn new(client: C) -> Self {
        Backend { client }
    }

    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        debug!("[backend.read]");

        let message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.client)).await??;

        debug!("message.code: {:?}", message.code as char);
        debug!("message: {:?}", message);

        match message.code.into() {
            Code::DataRow => {
                // debug!("DataRow");
            }
            Code::ErrorResponse => {
                // debug!("ErrorResponse");
            }
            Code::RowDescription => {
                // debug!("RowDescription");
            }
            code => {
                // debug!("Backend {code:?}");
            }
        }

        debug!("[backend.read] complete");
        Ok(message.bytes)
    }
}

impl From<u8> for Code {
    fn from(code: u8) -> Self {
        match code as char {
            'R' => Code::Authentication,
            'K' => Code::BackendKeyData,
            '2' => Code::BindComplete,
            '3' => Code::CloseComplete,
            'C' => Code::CommandComplete,
            'W' => Code::CopyBothResponse,
            'G' => Code::CopyInResponse,
            'H' => Code::CopyOutResponse,
            'D' => Code::DataRow,
            'I' => Code::EmptyQueryResponse,
            'E' => Code::ErrorResponse,
            'n' => Code::NoData,
            'N' => Code::NoticeResponse,
            'A' => Code::NotificationResponse,
            't' => Code::ParameterDescription,
            'S' => Code::ParameterStatus,
            '1' => Code::ParseComplete,
            's' => Code::PortalSuspended,
            'Z' => Code::ReadyForQuery,
            'T' => Code::RowDescription,
            _ => Code::Unknown(code as char),
        }
    }
}
