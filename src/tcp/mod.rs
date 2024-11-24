use std::time::Duration;

use socket2::TcpKeepalive;
use tokio::{
    net::{TcpListener, TcpStream},
    time,
};
use tracing::{debug, error, info, warn};

const INTERVAL: Duration = Duration::from_secs(5);
const TIME: Duration = Duration::from_secs(5);
const RETRIES: u32 = 5;

pub async fn bind_with_retry(addr: &str) -> TcpListener {
    let mut retry_count = 0;
    let max_retry_count = 10;
    let max_backoff = Duration::from_secs(2);

    loop {
        match TcpListener::bind(&addr).await {
            Ok(listener) => {
                return listener;
            }
            Err(err) => {
                if retry_count > max_retry_count {
                    error!("Error creating server connection after {max_retry_count} retries");
                    error!("{err}");
                    std::process::exit(exitcode::CONFIG);
                }
                info!("Attempting to create server connection");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        };
        let sleep_duration_ms = (100 * 2_u64.pow(retry_count)).min(max_backoff.as_millis() as _);
        time::sleep(Duration::from_millis(sleep_duration_ms)).await;

        retry_count += 1;
    }
}

///
/// Configure the tcp socket
///     set_nodelay
///     set_keepalive
///
/// TODO stay on target
/// TODO should this be configurable?
/// Keepalive is not as important without connection pooling timeouts to deal with
///
pub fn configure(stream: &TcpStream) {
    let sock_ref = socket2::SockRef::from(&stream);

    stream.set_nodelay(true).unwrap_or_else(|err| {
        warn!("Error configuring nodelay for connection");
        debug!("{err}");
    });

    match sock_ref.set_keepalive(true) {
        Ok(_) => {
            let params = &TcpKeepalive::new()
                .with_interval(INTERVAL)
                .with_retries(RETRIES)
                .with_time(TIME);

            match sock_ref.set_tcp_keepalive(params) {
                Ok(_) => (),
                Err(err) => {
                    warn!("Error configuring keepalive for connection");
                    debug!("{err}");
                }
            }
        }
        Err(err) => {
            warn!("Error configuring connection");
            debug!("{err}");
        }
    }
}

// pub fn configure_socket(stream: &TcpStream) {
//     let sock_ref = SockRef::from(stream);
//     let conf = get_config();

//     #[cfg(target_os = "linux")]
//     match sock_ref.set_tcp_user_timeout(Some(Duration::from_millis(conf.general.tcp_user_timeout)))
//     {
//         Ok(_) => (),
//         Err(err) => error!("Could not configure tcp_user_timeout for socket: {}", err),
//     }

//     sock_ref.set_nodelay(true).unwrap_or_else(|err| {
//         warn!("Could not configure nodelay for socket: {}", err);
//     });

//     match sock_ref.set_keepalive(true) {
//         Ok(_) => {
//             match sock_ref.set_tcp_keepalive(
//                 &TcpKeepalive::new()
//                     .with_interval(Duration::from_secs(conf.general.tcp_keepalives_interval))
//                     .with_retries(conf.general.tcp_keepalives_count)
//                     .with_time(Duration::from_secs(conf.general.tcp_keepalives_idle)),
//             ) {
//                 Ok(_) => (),
//                 Err(err) => error!("Could not configure tcp_keepalive for socket: {}", err),
//             }
//         }
//         Err(err) => error!("Could not configure socket: {}", err),
//     }
// }
