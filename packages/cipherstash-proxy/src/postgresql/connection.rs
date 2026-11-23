use crate::error::{Error, ErrorChain};
use crate::prometheus::CLIENTS_ACTIVE_CONNECTIONS;
use metrics::gauge;
use std::future::Future;
use tracing::{error, info, warn};

///
/// Runs a client connection handler and tracks it in the
/// `CLIENTS_ACTIVE_CONNECTIONS` gauge.
///
/// The gauge is incremented before the handler runs and decremented exactly once
/// after it returns, whether the connection ended successfully or with an error.
///
/// Errors returned by the handler are logged at a level that matches their cause.
///
pub async fn track_connection<F>(handler: F)
where
    F: Future<Output = Result<(), Error>>,
{
    gauge!(CLIENTS_ACTIVE_CONNECTIONS).increment(1);

    let result = handler.await;
    gauge!(CLIENTS_ACTIVE_CONNECTIONS).decrement(1);

    if let Err(err) = result {
        match err {
            Error::ConnectionClosed => {
                info!(msg = "Database connection closed by client");
            }
            Error::CancelRequest => {
                info!(msg = "Database connection closed after cancel request");
            }
            Error::ConnectionTimeout { .. } => {
                warn!(msg = "Database connection timeout", error = err.to_string());
            }
            _ => {
                error!(msg = "Database connection error", error = %ErrorChain(&err));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::{
        Counter, Gauge, GaugeFn, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
    };
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A change made to the active connections gauge.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum GaugeOp {
        Increment(f64),
        Decrement(f64),
    }

    /// Records every change made to the active connections gauge, in order.
    #[derive(Default)]
    struct ActiveConnections {
        ops: Mutex<Vec<GaugeOp>>,
    }

    impl ActiveConnections {
        fn ops(&self) -> Vec<GaugeOp> {
            self.ops.lock().unwrap().clone()
        }

        fn value(&self) -> f64 {
            self.ops().iter().fold(0.0, |value, op| match op {
                GaugeOp::Increment(n) => value + n,
                GaugeOp::Decrement(n) => value - n,
            })
        }
    }

    impl GaugeFn for ActiveConnections {
        fn increment(&self, value: f64) {
            self.ops.lock().unwrap().push(GaugeOp::Increment(value));
        }

        fn decrement(&self, value: f64) {
            self.ops.lock().unwrap().push(GaugeOp::Decrement(value));
        }

        fn set(&self, _: f64) {
            unimplemented!("track_connection never sets the active connections gauge");
        }
    }

    /// A recorder that captures only the active connections gauge.
    #[derive(Default)]
    struct TestRecorder {
        active_connections: Arc<ActiveConnections>,
    }

    impl Recorder for TestRecorder {
        fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

        fn register_counter(&self, _: &Key, _: &Metadata<'_>) -> Counter {
            Counter::noop()
        }

        fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
            if key.name() == CLIENTS_ACTIVE_CONNECTIONS {
                Gauge::from_arc(self.active_connections.clone())
            } else {
                Gauge::noop()
            }
        }

        fn register_histogram(&self, _: &Key, _: &Metadata<'_>) -> Histogram {
            Histogram::noop()
        }
    }

    const TRACKED_ONCE: [GaugeOp; 2] = [GaugeOp::Increment(1.0), GaugeOp::Decrement(1.0)];

    // `#[tokio::test]` runs on a current-thread runtime, so the thread-local recorder
    // installed by `set_default_local_recorder` sees every gauge change.

    mod given_handler_returns_ok {
        use super::*;

        #[tokio::test]
        async fn decrements_active_connections_once() {
            let recorder = TestRecorder::default();
            let _guard = metrics::set_default_local_recorder(&recorder);

            track_connection(async { Ok(()) }).await;

            assert_eq!(
                recorder.active_connections.ops(),
                TRACKED_ONCE,
                "active connections should be incremented once and decremented once"
            );
        }
    }

    mod given_handler_returns_an_error {
        use super::*;

        #[tokio::test]
        async fn decrements_active_connections_once() {
            let errors = [
                Error::ConnectionClosed,
                Error::CancelRequest,
                Error::ConnectionTimeout {
                    duration: Duration::from_millis(5000),
                },
                Error::DatabaseConnection,
            ];

            for err in errors {
                let description = format!("{err:?}");
                let recorder = TestRecorder::default();
                let _guard = metrics::set_default_local_recorder(&recorder);

                track_connection(async { Err(err) }).await;

                assert_eq!(
                    recorder.active_connections.ops(),
                    TRACKED_ONCE,
                    "active connections should be incremented once and decremented once for {description}"
                );
            }
        }
    }

    mod given_handler_is_running {
        use super::*;

        #[tokio::test]
        async fn counts_the_connection_as_active() {
            let recorder = TestRecorder::default();
            let _guard = metrics::set_default_local_recorder(&recorder);
            let observed = Cell::new(None);

            track_connection(async {
                observed.set(Some(recorder.active_connections.value()));
                Ok(())
            })
            .await;

            assert_eq!(
                observed.get(),
                Some(1.0),
                "active connections should count the connection while its handler runs"
            );
        }
    }
}
