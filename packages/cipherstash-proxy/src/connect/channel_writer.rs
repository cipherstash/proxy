use bytes::BytesMut;
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
            client_id,
        }
    }

    pub async fn receive(mut self) {
        while let Some(bytes) = self.receiver.recv().await {
            debug!(target: PROTOCOL,
                client_id = self.client_id,
                msg = "Writing",
                ?bytes
            );

            match self.writer.write_all(&bytes).await {
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
                }
            }
        }
    }

    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }
}
