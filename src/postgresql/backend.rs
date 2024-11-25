use crate::encrypt::Encrypt;
use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::{self};
use crate::postgresql::CONNECTION_TIMEOUT;
use bytes::{BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

const IS_SSL_REQUEST: bool = true;

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

pub struct Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    client: C,
    server: S,
    encrypt: Encrypt,
}

impl<C, S> Backend<C, S>
where
    C: AsyncWrite + Unpin,
    S: AsyncRead + Unpin,
{
    pub fn new(client: C, server: S, encrypt: Encrypt) -> Self {
        Backend {
            client,
            server,
            encrypt,
        }
    }

    ///
    /// Startup sequence:
    ///     Client: SSL Request
    ///     Server: SSL Response (single byte S or N)
    ///
    pub async fn read(&mut self) -> Result<BytesMut, Error> {
        // let code = self.client.read_u8().await?;
        // if !self.ssl_complete {
        //     if let Some(bytes) = self.ssl_request(code) {
        //         return Ok(bytes);
        //     }
        // }
        info!("[backend] read");
        let message =
            timeout(CONNECTION_TIMEOUT, protocol::read_message(&mut self.server)).await??;

        match message.code.into() {
            Code::Authentication => {}

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

    pub async fn write(&mut self, bytes: BytesMut) -> Result<(), Error> {
        self.client.write_all(&bytes).await?;
        Ok(())
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

// /// Handle TLS connection negotiation.
// pub async fn startup_tls(
//     stream: TcpStream,
//     client_server_map: ClientServerMap,
//     shutdown: Receiver<()>,
//     admin_only: bool,
//     tandem: Tandem,
// ) -> Result<Client<ReadHalf<TlsStream<TcpStream>>, WriteHalf<TlsStream<TcpStream>>>, Error> {
//     // Negotiate TLS.
//     let tls = Tls::new()?;
//     let addr = match stream.peer_addr() {
//         Ok(addr) => addr,
//         Err(err) => {
//             return Err(Error::SocketError(format!(
//                 "Failed to get peer address: {:?}",
//                 err
//             )));
//         }
//     };

//     let mut stream = match tls.acceptor.accept(stream).await {
//         Ok(stream) => stream,

//         // TLS negotiation failed.
//         Err(err) => {
//             error!("TLS negotiation failed: {:?}", err);
//             return Err(Error::TlsError);
//         }
//     };

//     // TLS negotiation successful.
//     // Continue with regular startup using encrypted connection.
//     match get_startup::<TlsStream<TcpStream>>(&mut stream).await {
//         // Got good startup message, proceeding like normal except we
//         // are encrypted now.
//         Ok((ClientConnectionType::Startup, bytes)) => {
//             let (read, write) = split(stream);

//             Client::startup(
//                 read,
//                 write,
//                 addr,
//                 bytes,
//                 client_server_map,
//                 shutdown,
//                 admin_only,
//                 tandem,
//             )
//             .await
//         }

//         // Bad Postgres client.
//         Ok((ClientConnectionType::Tls, _)) | Ok((ClientConnectionType::CancelQuery, _)) => {
//             Err(Error::ProtocolSyncError("Bad postgres client (tls)".into()))
//         }

//         Err(err) => Err(err),
//     }
// }
