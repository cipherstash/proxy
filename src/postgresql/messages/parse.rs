use super::{Destination, FrontendCode};
use crate::{
    error::{Error, ProtocolError},
    postgresql::protocol::BytesMutReadString,
    SIZE_I16, SIZE_I32,
};
use bytes::{Buf, BufMut, BytesMut};
use std::{ffi::CString, io::Cursor};

#[derive(Debug, Clone)]
pub struct Parse {
    pub code: char,
    pub len: i32,
    pub name: Destination,
    pub statement: String,
    pub num_params: i16,
    pub param_types: Vec<i32>,
    dirty: bool,
}

impl Parse {
    pub fn should_rewrite(&self) -> bool {
        self.dirty
    }
}

impl TryFrom<&BytesMut> for Parse {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Parse, Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8() as char;

        if FrontendCode::from(code) != FrontendCode::Parse {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::Parse.into(),
                received: code as char,
            }
            .into());
        }

        let len = cursor.get_i32();
        let name = cursor.read_string()?;
        let name = Destination::new(name);

        let statement = cursor.read_string()?;
        let num_params = cursor.get_i16();
        let mut param_types = Vec::new();

        for _ in 0..num_params {
            param_types.push(cursor.get_i32());
        }

        Ok(Parse {
            code,
            len,
            name,
            statement,
            num_params,
            param_types,
            dirty: false,
        })
    }
}

impl TryFrom<Parse> for BytesMut {
    type Error = Error;

    fn try_from(parse: Parse) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let name = CString::new(parse.name.as_str())?;
        let name = name.as_bytes_with_nul();

        let statement = CString::new(parse.statement)?;
        let statement = statement.as_bytes_with_nul();

        let len = SIZE_I32 // len
                + name.len()
                + statement.len()
                + SIZE_I16 // num_params
                + SIZE_I32 * parse.param_types.len();

        bytes.put_u8(FrontendCode::Parse.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(name);
        bytes.put_slice(statement);
        bytes.put_i16(parse.num_params);
        for param in parse.param_types {
            bytes.put_i32(param);
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {

    use tracing::error;

    use crate::{postgresql::messages::parse::Parse, trace};

    use super::Destination;

    #[test]
    fn test_parse_destination() {
        trace();

        let name = "test".to_string();
        let destination = Destination::new(name);

        assert_eq!(destination.as_str(), "test");

        let name = "".to_string();
        let destination = Destination::new(name);

        assert_eq!(destination.as_str(), "");
    }
}
