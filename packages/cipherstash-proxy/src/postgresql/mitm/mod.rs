mod messages;
mod state;

use bytes::BytesMut;
use tokio::io::{ReadHalf, WriteHalf};

use crate::{
    connect::AsyncStream,
    error::Error,
    postgresql::{
        mitm::state::Ready,
        protocol,
    },
    Encrypt,
};

struct Session<State, CMM, SMM> {
    state: State,
    client_id: i32,
    encrypt: Encrypt,
    client_reader: ReadHalf<AsyncStream>,
    client_writer: WriteHalf<AsyncStream>,
    client_message_mapper: CMM,
    server_reader: ReadHalf<AsyncStream>,
    server_writer: WriteHalf<AsyncStream>,
    server_message_mapper: SMM,
}

trait SessionState {
    type ExpectedMessage: TryFrom<(char, BytesMut), Error = Error>;
}

pub(crate) struct SessionData {}

impl<State: SessionState, CMM, SMM> Session<State, CMM, SMM> {
    async fn next_message(&mut self) -> Result<State::ExpectedMessage, Error> {
        let connection_timeout = self.encrypt.config.database.connection_timeout();
        let (code, bytes) =
            protocol::read_message(&mut self.client_reader, self.client_id, connection_timeout)
                .await?;

        Ok(State::ExpectedMessage::try_from((code as char, bytes))?)
    }
}

impl<CMM, SMM> Session<Ready, CMM, SMM> {
    pub(crate) fn ready(
        encrypt: Encrypt,
        client_id: i32,
        client: AsyncStream,
        client_message_mapper: CMM,
        server: AsyncStream,
        server_message_mapper: SMM,
    ) -> Self {
        let (client_reader, client_writer) = client.split();
        let (server_reader, server_writer) = server.split();

        Self {
            state: Ready,
            encrypt,
            client_id,
            client_reader,
            client_writer,
            client_message_mapper,
            server_reader,
            server_writer,
            server_message_mapper,
        }
    }
}
