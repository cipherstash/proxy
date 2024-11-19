use crate::error::Error;
use crate::postgresql::{read_message, CONNECTION_TIMEOUT};

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

        let mut message = timeout(CONNECTION_TIMEOUT, read_message(&mut self.client)).await??;

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

        Ok(message.bytes)
    }
}

// async fn handle_error(
//     &mut self,
//     cursor: &mut Cursor<BytesMut>,
//     mut output: BytesMut,
// ) -> Result<BytesMut, Error> {
//     let error_response = ErrorResponse::try_from(cursor)?;
//     error!("{error_response}");
//     self.complete_log
//         .set_statement_error(format!("{error_response}"));
//     output = self.flush(output).await?;
//     error_response.write_into(output)
// }

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
