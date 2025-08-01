use bytes::BytesMut;
use tokio::io::{ReadHalf, WriteHalf};

use crate::{
    connect::AsyncStream,
    error::Error,
    postgresql::messages::{BackendCode, FrontendCode},
};

struct Mitm {
    client_reader: ReadHalf<AsyncStream>,
    client_writer: WriteHalf<AsyncStream>,
    client_message_transformer: fn(ClientMessage) -> ClientMessage,
    server_reader: ReadHalf<AsyncStream>,
    server_writer: WriteHalf<AsyncStream>,
    server_message_transformer: fn(ServerMessage) -> ServerMessage,
}

pub(crate) struct ClientMessage(FrontendCode, BytesMut);
pub(crate) struct ServerMessage(BackendCode, BytesMut);

enum Message {
    ClientMessage(ClientMessage),
    ServerMessage(ServerMessage),
}

pub(crate) const NOOP_CLIENT_MESSAGE_TRANSFORMER: fn(ClientMessage) -> ClientMessage =
    |message| message;

pub(crate) const NOOP_SERVER_MESSAGE_TRANSFORMER: fn(ServerMessage) -> ServerMessage =
    |message| message;

impl Mitm {
    pub(crate) fn new(
        client: AsyncStream,
        client_message_transformer: fn(ClientMessage) -> ClientMessage,
        server: AsyncStream,
        server_message_transformer: fn(ServerMessage) -> ServerMessage,
    ) -> Self {
        let (client_reader, client_writer) = client.split();
        let (server_reader, server_writer) = server.split();

        Self {
            client_reader,
            client_writer,
            client_message_transformer,
            server_reader,
            server_writer,
            server_message_transformer,
        }
    }

    pub(crate) async fn message_loop(&mut self) -> Result<(), Error> {

    }
}
