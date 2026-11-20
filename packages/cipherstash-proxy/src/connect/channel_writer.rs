use bytes::BytesMut;
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use tracing::{debug, error};

use crate::log::PROTOCOL;

pub type Receiver = UnboundedReceiver<BytesMut>;
pub type Sender = UnboundedSender<BytesMut>;

#[derive(Debug)]
pub struct ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    writer: W,
    receiver: Receiver,
    sender: Sender,
    bytes_written: Arc<AtomicU64>,
    client_id: i32,
}

impl<W> ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    pub fn new(writer: W, client_id: i32) -> Self {
        let (sender, receiver): (UnboundedSender<BytesMut>, UnboundedReceiver<BytesMut>) =
            mpsc::unbounded_channel();

        ChannelWriter {
            writer,
            receiver,
            sender,
            bytes_written: Arc::new(AtomicU64::new(0)),
            client_id,
        }
    }

    pub async fn receive(mut self) {
        debug!(target: PROTOCOL,
            client_id = self.client_id,
            msg = "ChannelWriter task started",
        );

        // Drop our own sender so the channel can close when frontend/backend senders are dropped
        // Without this, we have a circular dependency: receiver waits for all senders to drop,
        // but we're holding one of them ourselves!
        drop(self.sender);

        while let Some(bytes) = self.receiver.recv().await {
            debug!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Writing",
                ?bytes
            );

            match write_all(&mut self.writer, &self.bytes_written, &bytes).await {
                Ok(_) => {
                    debug!(target: PROTOCOL,
                        client_id = self.client_id,
                        msg = "Write complete",
                    );
                }
                Err(err) => {
                    error!(target: PROTOCOL,
                        client_id = self.client_id,
                        msg = "Write error",
                        error = ?err
                    );
                    break;
                }
            }
        }

        // Channel closed - shutdown the writer to properly close the connection
        debug!(target: PROTOCOL,
            client_id = self.client_id,
            msg = "Recv loop exited - channel closed, beginning shutdown",
        );

        // The connection is ending, and the usual cause of a failure from here on is a client that
        // has already gone, for example one that crashed or was killed. There is nothing to act
        // on, so these failures are logged at debug level.

        // Flush any pending writes before shutdown
        if let Err(err) = self.writer.flush().await {
            debug!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Error flushing writer during shutdown",
                error = ?err
            );
        }

        // Shutdown the write half to send FIN and properly close the connection
        if let Err(err) = self.writer.shutdown().await {
            debug!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Error shutting down writer",
                error = ?err
            );
        }

        debug!(target: PROTOCOL,
            client_id = self.client_id,
            msg = "Writer shutdown complete",
        );
    }

    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// A count of the bytes the writer has accepted so far.
    ///
    /// The count goes up each time the writer accepts some bytes, even part of a message, so it
    /// shows whether a client is still reading. It stops going up when the client stops reading
    /// and the buffers between Proxy and the client are full.
    pub fn bytes_written(&self) -> Arc<AtomicU64> {
        self.bytes_written.clone()
    }
}

/// Writes all of `bytes`, adding to `bytes_written` as the writer accepts them
async fn write_all<W>(writer: &mut W, bytes_written: &AtomicU64, mut bytes: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while !bytes.is_empty() {
        let written = writer.write(bytes).await?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes_written.fetch_add(written as u64, Ordering::Relaxed);
        bytes = &bytes[written..];
    }
    Ok(())
}
