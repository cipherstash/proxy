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

// 1 minute
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1000 * 1);

// Used in the StartupMessage to indicate regular handshake.
const PROTOCOL_VERSION_NUMBER: i32 = 196608;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection closed by client")]
    ConnectionClosed,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error("Connection timed out")]
    Timeout(#[from] Elapsed),
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

#[tokio::main]
async fn main() {
    trace();

    let listener = TcpListener::bind("127.0.0.1:6432").await.unwrap();
    println!("Server listening on 127.0.0.1:6432");

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
                            Error::Timeout(_) => {
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
    let mut server = TcpStream::connect("127.0.0.1:5432").await?;

    let (client_reader, mut client_writer) = client.split();
    let (mut server_reader, mut server_writer) = server.split();

    let mut pg = PostgreSQL::new(client_reader);

    let client_to_server = async {
        loop {
            let bytes = pg.read().await?;
            debug!("[handle]");
            debug!("bytes: {bytes:?}");

            server_writer.write_all(&bytes).await?;
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

pub trait Read {
    // fn read<'a>(
    //     &'a mut self,
    // ) -> Pin<Box<dyn Future<Output = Result<BytesMut, anyhow::Error>> + Send + 'a>>;
    // async fn read(&mut self) -> Result<BytesMut, anyhow::Error>;
    fn read(&mut self)
        -> impl std::future::Future<Output = Result<BytesMut, anyhow::Error>> + Send;
}
