mod postgresql;

use bytes::BytesMut;
use postgresql::PostgreSQL;
use serde::{Deserialize, Serialize};
use std::mem;
use std::sync::Once;
use thiserror::Error;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::error::Elapsed;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

static INIT: Once = Once::new();

const SIZE_U8: usize = mem::size_of::<u8>();
const SIZE_I16: usize = mem::size_of::<i16>();
const SIZE_I32: usize = mem::size_of::<i32>();

// const URL: &str = "127.0.0.1:6432";
// const DOWNSTREAM: &str = "127.0.0.1:5432";

const URL: &str = "127.0.0.1:6433";
const DOWNSTREAM: &str = "127.0.0.1:1433";

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection closed by client")]
    ConnectionClosed,

    #[error("Connection timed out")]
    ConnectionTimeout(#[from] Elapsed),

    #[error(transparent)]
    Io(io::Error),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterFormatCodesMismatch { expected: usize, received: usize },

    #[error("Expected {expected} parameter format codes, received {received}")]
    ParameterResultFormatCodesMismatch { expected: usize, received: usize },

    #[error("Unexpected message length {len} for code {code}")]
    UnexpectedMessageLength { code: u8, len: usize },

    #[error("Unexpected null in string")]
    UnexpectedNull,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::UnexpectedEof => Error::ConnectionClosed,
            _ => Error::Io(e),
        }
    }
}

pub trait Read {
    fn read(&mut self) -> impl std::future::Future<Output = Result<BytesMut, Error>> + Send;
}

#[tokio::main]
async fn main() {
    trace();

    let listener = TcpListener::bind(URL).await.unwrap();
    info!(url = URL, "Server listening");

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            loop {
                match handle(&mut socket).await {
                    Ok(_) => (),
                    Err(e) => {
                        match e {
                            Error::ConnectionClosed => {
                                info!("Connection closed by client");
                            }
                            Error::ConnectionTimeout(_) => {
                                warn!("Connection timeout");
                            }
                            _ => {
                                error!("Error {:?}", e);
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
}

async fn handle(client: &mut TcpStream) -> Result<(), Error> {
    let mut server = TcpStream::connect(DOWNSTREAM).await?;

    info!(database = DOWNSTREAM, "Connected");

    let (mut client_reader, mut client_writer) = client.split();
    let (mut server_reader, mut server_writer) = server.split();

    let mut pg = PostgreSQL::new(client_reader);

    let client_to_server = async {
        loop {
            let bytes = pg.read().await?;

            debug!("[client_to_server]");
            debug!("bytes: {bytes:?}");

            server_writer.write_all(&bytes).await?;
            debug!("write complete");
        }
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        io::copy(&mut server_reader, &mut client_writer).await?;
        Ok::<(), Error>(())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}

fn trace() {
    INIT.call_once(|| {
        use tracing_subscriber::FmtSubscriber;

        let subscriber = FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG) // Set the maximum level of tracing events that should be logged.
            .with_line_number(true)
            .with_target(true)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}
