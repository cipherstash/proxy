use pg_proto::{
    codec::{BackendMessage, Frontend},
    transport::Buffered,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use tracing::{debug, error};

use crate::log::PROTOCOL;

pub type Receiver = UnboundedReceiver<BackendMessage>;
pub type Sender = UnboundedSender<BackendMessage>;

#[derive(Debug)]
pub struct ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    writer: Buffered<W, Frontend>,
    receiver: Receiver,
    sender: Sender,
    client_id: i32,
}

impl<W> ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    pub fn new(writer: W, client_id: i32) -> Self {
        let (sender, receiver): (
            UnboundedSender<BackendMessage>,
            UnboundedReceiver<BackendMessage>,
        ) = mpsc::unbounded_channel();

        ChannelWriter {
            writer: Buffered::new_frontend(writer),
            receiver,
            sender,
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

        while let Some(message) = self.receiver.recv().await {
            debug!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Writing",
                ?message
            );

            let result = message.to_frame().and_then(|frame| self.writer.push(frame));
            let result = match result {
                Ok(()) => self.writer.flush().await,
                Err(error) => Err(error),
            };
            match result {
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

        // Flush any pending writes before shutdown
        if let Err(err) = self.writer.flush().await {
            error!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Error flushing writer during shutdown",
                error = ?err
            );
        }

        // Shutdown the write half to send FIN and properly close the connection
        if let Err(err) = self.writer.into_inner().shutdown().await {
            error!(target: PROTOCOL,
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
}
