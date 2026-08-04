use bytes::BytesMut;
use pg_proto::codec::{Backend, BackendMessage, Frontend, FrontendMessage, PgCodec};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::Error;

pub fn decode_frontend_frame(bytes: &BytesMut) -> Result<FrontendMessage, Error> {
    let mut bytes = bytes.clone();
    PgCodec::<Frontend>::default()
        .decode(&mut bytes)?
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
}

pub fn decode_backend_frame(bytes: &BytesMut) -> Result<BackendMessage, Error> {
    let mut bytes = bytes.clone();
    PgCodec::<Backend>::default()
        .decode(&mut bytes)?
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
}

pub fn encode_frontend_message(message: &FrontendMessage) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();
    PgCodec::<Frontend>::default().encode(message.to_frame()?, &mut bytes)?;
    Ok(bytes)
}

pub fn encode_backend_message(message: &BackendMessage) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();
    PgCodec::<Backend>::default().encode(message.to_frame()?, &mut bytes)?;
    Ok(bytes)
}
