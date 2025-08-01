use bytes::BytesMut;
use tokio::io::{ReadHalf, WriteHalf};

use crate::{
    connect::AsyncStream, error::Error, postgresql::messages::{BackendCode, FrontendCode}
};

struct Mitm<FC, FS> {
    client_reader: ReadHalf<AsyncStream>,
    client_writer: WriteHalf<AsyncStream>,
    client_message_transformer: FC,
    server_reader: ReadHalf<AsyncStream>,
    server_writer: WriteHalf<AsyncStream>,
    server_message_transformer: FS,
}

pub(crate) struct ClientMessage(FrontendCode, BytesMut);
pub(crate) struct ServerMessage(BackendCode, BytesMut);

enum Message {
    Client(ClientMessage),
    Server(ServerMessage),
}

pub(crate) const NOOP_CLIENT_MESSAGE_TRANSFORMER: fn(ClientMessage) -> ClientMessage =
    |message: ClientMessage| message;

pub(crate) const NOOP_SERVER_MESSAGE_TRANSFORMER: fn(ServerMessage) -> ServerMessage =
    |message: ServerMessage| message;

impl<FC, FS> Mitm<FC, FS>
where
    FC: Fn(ClientMessage) -> ClientMessage,
    FS: Fn(ServerMessage) -> ServerMessage,
{
    pub(crate) fn new(
        client: AsyncStream,
        client_message_transformer: FC,
        server: AsyncStream,
        server_message_transformer: FS,
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
