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

    match result {
        Ok(_) => (),
        Err(err) => match err {
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
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use metrics::{
        Counter, Gauge, GaugeFn, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
    };
    use std::sync::{Arc, Mutex};

    /// A change made to the active connections gauge.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum GaugeOp {
        Increment(f64),
        Decrement(f64),
        Set(f64),
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
                GaugeOp::Set(n) => *n,
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

        fn set(&self, value: f64) {
            self.ops.lock().unwrap().push(GaugeOp::Set(value));
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

    /// Drives `future` to completion with `recorder` installed as the metrics recorder.
    ///
    /// The local recorder is thread-local, so the future runs on a current-thread
    /// runtime inside the recorder's scope.
    fn run_with_recorder<F: Future>(recorder: &TestRecorder, future: F) -> F::Output {
        metrics::with_local_recorder(recorder, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(future)
        })
    }

    mod given_handler_returns_ok {
        use super::*;

        #[test]
        fn returns_active_connections_to_start() {
            let recorder = TestRecorder::default();

            run_with_recorder(&recorder, track_connection(async { Ok(()) }));

            assert_eq!(
                recorder.active_connections.value(),
                0.0,
                "active connections should return to zero after a successful connection, ops: {:?}",
                recorder.active_connections.ops()
            );
        }
    }

    mod given_handler_returns_connection_closed {
        use super::*;

        #[test]
        fn decrements_active_connections_once() {
            let recorder = TestRecorder::default();

            run_with_recorder(
                &recorder,
                track_connection(async { Err(Error::ConnectionClosed) }),
            );

            assertions::tracked_once(&recorder);
        }
    }

    mod given_handler_returns_connection_timeout {
        use super::*;
        use std::time::Duration;

        #[test]
        fn decrements_active_connections_once() {
            let recorder = TestRecorder::default();

            run_with_recorder(
                &recorder,
                track_connection(async {
                    Err(Error::ConnectionTimeout {
                        duration: Duration::from_millis(5000),
                    })
                }),
            );

            assertions::tracked_once(&recorder);
        }
    }

    mod given_handler_returns_other_error {
        use super::*;

        #[test]
        fn decrements_active_connections_once() {
            let recorder = TestRecorder::default();

            run_with_recorder(
                &recorder,
                track_connection(async { Err(Error::DatabaseConnection) }),
            );

            assertions::tracked_once(&recorder);
        }
    }

    mod assertions {
        use super::*;

        /// The connection incremented the gauge once and decremented it once.
        #[track_caller]
        pub(super) fn tracked_once(recorder: &TestRecorder) {
            assert_eq!(
                recorder.active_connections.ops(),
                vec![GaugeOp::Increment(1.0), GaugeOp::Decrement(1.0)],
                "active connections should be incremented once and decremented once"
            );
        }
    }
}
