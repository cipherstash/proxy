use crate::{client_message, error::Error};

client_message!(Bind 'B');
client_message!(Describe 'D');
client_message!(Execute 'E');
client_message!(PasswordMessage 'p');
client_message!(Parse 'P');
client_message!(Query 'Q');
client_message!(Sync 'S');
client_message!(Terminate 'X');

pub trait ClientMessageMapper {
    fn map_bind(&self, message: Bind) -> Result<Bind, Error>;
    fn map_describe(&self, message: Describe) -> Result<Describe, Error>;
    fn map_execute(&self, message: Execute) -> Result<Execute, Error>;
    fn map_password_message(&self, message: PasswordMessage) -> Result<PasswordMessage, Error>;
    fn map_parse(&self, message: Parse) -> Result<Parse, Error>;
    fn map_query(&self, message: Query) -> Result<Query, Error>;
    fn map_sync(&self, message: Sync) -> Result<Sync, Error>;
    fn map_terminate(&self, message: Terminate) -> Result<Terminate, Error>;
}

pub struct ClientMessageNoopMapper;

pub static NOOP_CLIENT_MSG_MAPPER: ClientMessageNoopMapper = ClientMessageNoopMapper;

impl ClientMessageMapper for ClientMessageNoopMapper {
    fn map_bind(&self, message: Bind) -> Result<Bind, Error> {
        Ok(message)
    }

    fn map_describe(&self, message: Describe) -> Result<Describe, Error> {
        Ok(message)
    }

    fn map_execute(&self, message: Execute) -> Result<Execute, Error> {
        Ok(message)
    }

    fn map_password_message(&self, message: PasswordMessage) -> Result<PasswordMessage, Error> {
        Ok(message)
    }

    fn map_parse(&self, message: Parse) -> Result<Parse, Error> {
        Ok(message)
    }

    fn map_query(&self, message: Query) -> Result<Query, Error> {
        Ok(message)
    }

    fn map_sync(&self, message: Sync) -> Result<Sync, Error> {
        Ok(message)
    }

    fn map_terminate(&self, message: Terminate) -> Result<Terminate, Error> {
        Ok(message)
    }
}