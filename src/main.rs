use my_little_proxy::config::TandemConfig;
use my_little_proxy::encrypt::Encrypt;
use my_little_proxy::error::Error;
use my_little_proxy::{postgresql, trace};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

// TODO Add to config
const URL: &str = "127.0.0.1:6432";

// TODO: Accept command line arguments for config file path
#[tokio::main]
async fn main() {
    let config_file = "cipherstash-proxy.toml";

    trace();

    let config = match TandemConfig::load(config_file) {
        Ok(config) => config,
        Err(err) => {
            error!("Configuration Error: {}", err);
            std::process::exit(exitcode::CONFIG);
        }
    };

    let encrypt = startup(config).await;

    let listener = TcpListener::bind(URL).await.unwrap();
    info!(url = URL, "Server listening");

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let encrypt = encrypt.clone();
        tokio::spawn(async move {
            loop {
                match handle(encrypt.clone(), &mut socket).await {
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

async fn startup(config: TandemConfig) -> Encrypt {
    if config.encrypt.dataset_id.is_none() {
        info!("Encrypt: using default dataset");
    }

    match Encrypt::init(config).await {
        Ok(encrypt) => encrypt,
        Err(err) => {
            error!("Failed to initialise : {}", err);
            std::process::exit(exitcode::UNAVAILABLE);
        }
    }
}

async fn handle(encrypt: Encrypt, client: &mut TcpStream) -> Result<(), Error> {
    let mut server = TcpStream::connect(&encrypt.config.connect.to_socket_address()).await?;

    info!(
        database = encrypt.config.connect.to_socket_address(),
        "Connected"
    );

    let (client_reader, mut client_writer) = client.split();
    let (server_reader, mut server_writer) = server.split();

    let client_to_server = async {
        let mut fe = postgresql::Frontend::new(client_reader, server_writer, encrypt);
        loop {
            let bytes = fe.read().await?;

            // debug!("[client_to_server]");
            // debug!("bytes: {bytes:?}");

            fe.write(bytes).await?;
            // debug!("write complete");
        }

        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        let mut be = postgresql::Backend::new(server_reader);

        loop {
            let bytes = be.read().await?;

            // debug!("[client_to_server]");
            // debug!("bytes: {bytes:?}");

            client_writer.write_all(&bytes).await?;
            // debug!("write complete");
        }

        Ok::<(), Error>(())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
