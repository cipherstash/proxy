use bytes::BytesMut;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc::{self, Receiver, Sender},
};
use tracing::{debug, error};

use crate::log::PROTOCOL;

#[derive(Debug)]
pub struct ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    writer: W,
    receiver: Receiver<BytesMut>,
    sender: Sender<BytesMut>,
    client_id: i32,
}

impl<W> ChannelWriter<W>
where
    W: AsyncWrite + Unpin,
{
    pub fn new(writer: W, client_id: i32) -> Self {
        let (sender, receiver): (Sender<BytesMut>, Receiver<BytesMut>) = mpsc::channel(32);

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

    pub fn sender(&self) -> Sender<BytesMut> {
        self.sender.clone()
    }
}
