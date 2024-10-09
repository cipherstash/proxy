use crate::{Error, Read, SIZE_I16, SIZE_I32, SIZE_U8};
use bytes::{BufMut, BytesMut};
use tokio::io::{self, AsyncRead, AsyncReadExt};
use tracing::debug;

pub struct SQLServer<C>
where
    C: AsyncRead + Unpin + Send,
{
    client: C,
}

///
/// SQL Server Message Header
///
/// +---------+--------+--------+--------+--------+--------+
/// | Type    | Status | Length | SPID   | Packet | Window |
/// +---------+--------+--------+--------+--------+--------+
/// | 1 byte  | 1 byte | 2 bytes| 2 bytes| 1 byte | 1 byte |
/// +---------+--------+--------+--------+--------+--------+
///

impl<C> Read for SQLServer<C>
where
    C: AsyncRead + Unpin + Send,
{
    fn read(&mut self) -> impl std::future::Future<Output = Result<BytesMut, Error>> + Send {
        debug!("[SQLServer.read]");
        async move {
            let message_type = self.client.read_u8().await?;
            debug!("message_type {message_type}");
            debug!("message_type {}", message_type as char);

            let status = self.client.read_u8().await?;
            debug!("status {status}");

            let len = self.client.read_i16().await?;
            debug!("len {len}");

            // len includes the total length of the packet, including the 8-byte header
            let capacity = len as usize;
            let mut bytes = BytesMut::with_capacity(capacity);

            bytes.put_u8(message_type);
            bytes.put_u8(status);
            bytes.put_i16(len);

            debug!("bytes {bytes:?}");

            let slice_start = bytes.len();

            // debug!("slice_start {slice_start}");
            // let x = SIZE_U8 + SIZE_U8 + SIZE_I16;
            // debug!("SIZE_U8 + SIZE_U8 + SIZE_I16 {x}");

            // Capacity and len are not the same!!
            // resize populates the buffer with 0s
            bytes.resize(capacity, b'0');
            self.client.read_exact(&mut bytes[slice_start..]).await?;

            debug!("bytes {bytes:?}");

            Ok(bytes)
        }
    }
}

impl<C> SQLServer<C>
where
    C: AsyncRead + Unpin + Send,
{
    pub fn new(client: C) -> Self {
        SQLServer { client }
    }
}
