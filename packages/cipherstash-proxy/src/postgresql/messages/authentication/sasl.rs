use std::{
    ffi::CString,
    io::{Cursor, Read},
};

use bytes::{Buf, BufMut, BytesMut};

use crate::{
    error::{Error, ProtocolError},
    postgresql::{messages::FrontendCode, protocol::BytesMutReadString},
    SIZE_I32,
};

use super::auth::{self, SaslMechanism};

#[derive(Clone, Debug)]
pub struct SASLInitialResponse {
    #[allow(dead_code)]
    code: u8,
    pub mechanism: String,
    pub response: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SASLResponse {
    #[allow(dead_code)]
    code: u8,
    response: Vec<u8>,
}

impl SASLInitialResponse {
    pub fn new(mechanism: SaslMechanism, response: Vec<u8>) -> Self {
        let mechanism = mechanism.to_string();

        SASLInitialResponse {
            code: FrontendCode::SASLInitialResponse.into(),
            mechanism,

            response,
        }
    }

    pub fn is_scram_sha_256(&self) -> bool {
        self.mechanism == auth::SCRAM_SHA_256
    }

    pub fn is_scram_sha_256_plus(&self) -> bool {
        self.mechanism == auth::SCRAM_SHA_256_PLUS
    }
}

impl SASLResponse {
    pub fn new(response: Vec<u8>) -> Self {
        SASLResponse {
            code: FrontendCode::SASLResponse.into(),
            response,
        }
    }
}

impl TryFrom<&BytesMut> for SASLInitialResponse {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<SASLInitialResponse, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        // Note: all password messages use the 'p' code
        if code != b'p' {
            return Err(ProtocolError::UnexpectedMessageCode1 {
                expected: FrontendCode::SASLInitialResponse.into(),
                received: code as char,
            }
            .into());
        }
        let _len = cursor.get_i32();
        let mechanism = cursor.read_string()?;
        let _response_len = cursor.get_i32();
        let mut bytes = Vec::new();
        cursor.read_to_end(&mut bytes)?;

        Ok(SASLInitialResponse {
            code,
            mechanism,
            response: bytes,
        })
    }
}

impl TryFrom<SASLInitialResponse> for BytesMut {
    type Error = Error;

    fn try_from(response: SASLInitialResponse) -> Result<BytesMut, Self::Error> {
        let mut bytes = BytesMut::new();

        let mechanism = CString::new(response.mechanism)?;
        let mechanism = mechanism.as_bytes_with_nul();

        let response_len = response.response.len();

        let len = SIZE_I32         // len length
                        + mechanism.len()
                        + SIZE_I32        // response_len
                        + response_len;

        bytes.put_u8(FrontendCode::SASLInitialResponse.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(mechanism);
        bytes.put_i32(response_len as i32);
        bytes.put_slice(&response.response);

        Ok(bytes)
    }
}

impl TryFrom<SASLResponse> for BytesMut {
    type Error = Error;

    fn try_from(response: SASLResponse) -> Result<BytesMut, Self::Error> {
        let mut bytes = BytesMut::new();

        let response_len = response.response.len();

        let len = SIZE_I32         // len length
                        + response_len;

        bytes.put_u8(FrontendCode::SASLResponse.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(&response.response);

        Ok(bytes)
    }
}
