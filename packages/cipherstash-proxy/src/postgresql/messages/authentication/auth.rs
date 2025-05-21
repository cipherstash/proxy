use crate::error::{Error, ProtocolError};
use crate::postgresql::messages::{BackendCode, FrontendCode};
use crate::postgresql::protocol::BytesMutReadString;
use crate::SIZE_I32;
use bytes::{Buf, BufMut, BytesMut};

use std::convert::TryFrom;
use std::ffi::CString;
use std::fmt::{self, Display, Formatter};
use std::io::{Cursor, Read};

const SIZE_NULL_BYTE: usize = 1;

pub const SCRAM_SHA_256_PLUS: &str = "SCRAM-SHA-256-PLUS";
pub const SCRAM_SHA_256: &str = "SCRAM-SHA-256";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaslMechanism {
    ScramSha256,
    ScramSha256Plus,
}

#[derive(Debug, Clone)]
pub struct Authentication {
    #[allow(dead_code)]
    code: u8,
    pub method: AuthenticationMethod,
}

#[derive(Clone, Debug)]
#[repr(i32)]
pub enum AuthenticationMethod {
    AuthenticationOk = 0,
    AuthenticationCleartextPassword = 3,
    Md5Password { salt: [u8; 4] } = 5,
    Sasl { mechanisms: Vec<SaslMechanism> } = 10,
    AuthenticationSASLContinue { bytes: Vec<u8> } = 11,
    AuthenticationSASLFinal { bytes: Vec<u8> } = 12,
    Other { method_code: i32, bytes: Vec<u8> },
}

#[derive(Clone, Debug)]
pub struct PasswordMessage {
    code: u8,
    pub password: String,
}

impl Authentication {
    pub fn is_ok(&self) -> bool {
        matches!(self.method, AuthenticationMethod::AuthenticationOk)
    }

    pub fn is_sasl(&self) -> bool {
        matches!(self.method, AuthenticationMethod::Sasl { .. })
    }

    pub fn is_scram_sha_256_plus(&self) -> bool {
        match self.method {
            AuthenticationMethod::Sasl { ref mechanisms } => {
                mechanisms.contains(&SaslMechanism::ScramSha256Plus)
            }
            _ => false,
        }
    }

    ///
    /// Returns the first mechanism in the list of mechanisms
    /// If the method is not SASL, it will return an error
    /// If the method is SASL and there are no mechanisms, it will return an error
    ///   - This should never happen as the server should always return at least one mechanism
    ///   - If it does, it is a protocol error and the message parse should already have returned an error
    ///   - This is a safety check to ensure that the server is behaving as expected
    ///
    pub fn sasl_mechanism(&self) -> Result<SaslMechanism, Error> {
        let mechanism = match self.method {
            AuthenticationMethod::Sasl { ref mechanisms } => mechanisms.first(),
            _ => None,
        };

        match mechanism {
            Some(m) => Ok(*m),
            None => {
                Err(ProtocolError::UnexpectedSaslAuthenticationMethod("None".to_string()).into())
            }
        }
    }

    pub fn sasl_continue(&self) -> Result<&Vec<u8>, Error> {
        match self.method {
            AuthenticationMethod::AuthenticationSASLContinue { ref bytes } => Ok(bytes),
            _ => Err(ProtocolError::UnexpectedAuthenticationResponse {
                expected: "SASLContinue".into(),
                received: (&self.method).into(),
            }
            .into()),
        }
    }

    pub fn sasl_final(&self) -> Result<&Vec<u8>, Error> {
        match self.method {
            AuthenticationMethod::AuthenticationSASLFinal { ref bytes } => Ok(bytes),
            _ => Err(ProtocolError::UnexpectedAuthenticationResponse {
                expected: "SASLFinal".into(),
                received: (&self.method).into(),
            }
            .into()),
        }
    }

    pub fn md5_password(salt: [u8; 4]) -> Authentication {
        Authentication {
            code: BackendCode::Authentication.into(),
            method: AuthenticationMethod::Md5Password { salt },
        }
    }

    pub fn authentication_ok() -> Authentication {
        Authentication {
            code: BackendCode::Authentication.into(),
            method: AuthenticationMethod::AuthenticationOk,
        }
    }
}

impl PasswordMessage {
    pub fn new(password: String) -> PasswordMessage {
        PasswordMessage {
            code: FrontendCode::PasswordMessage.into(),
            password,
        }
    }
}

impl TryFrom<&BytesMut> for Authentication {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<Authentication, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if BackendCode::from(code) != BackendCode::Authentication {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: BackendCode::Authentication.into(),
                received: code as char,
            }
            .into());
        }

        let len = cursor.get_i32(); // read and progress cursor
        let method_code = cursor.get_i32();

        let method = match method_code {
            0 => AuthenticationMethod::AuthenticationOk,
            5 => {
                let mut salt = [0; 4];
                cursor.read_exact(&mut salt)?;
                AuthenticationMethod::Md5Password { salt }
            }
            10 => {
                let mut mechanisms = Vec::new();
                let mut count = SIZE_I32        // message len
                                     + SIZE_I32        // method_code
                                     + SIZE_NULL_BYTE; // terminating null byte;

                while count < (len as usize) {
                    let m = cursor.read_string()?;
                    count += m.len() + SIZE_NULL_BYTE;
                    mechanisms.push(SaslMechanism::try_from(m)?);
                }
                AuthenticationMethod::Sasl { mechanisms }
            }
            11 => {
                let mut bytes = Vec::new();
                cursor.read_to_end(&mut bytes)?;
                AuthenticationMethod::AuthenticationSASLContinue { bytes }
            }
            12 => {
                let mut bytes = Vec::new();
                cursor.read_to_end(&mut bytes)?;
                AuthenticationMethod::AuthenticationSASLFinal { bytes }
            }
            _ => {
                // Get any remaining bytes from the cursor
                let mut bytes = Vec::new();
                cursor.read_to_end(&mut bytes)?;
                AuthenticationMethod::Other { method_code, bytes }
            }
        };

        Ok(Authentication { code, method })
    }
}

impl TryFrom<Authentication> for BytesMut {
    type Error = Error;

    fn try_from(auth: Authentication) -> Result<BytesMut, Error> {
        let mut method_bytes = BytesMut::new();

        let method_code = (&auth.method).into();
        method_bytes.put_i32(method_code);

        match auth.method {
            AuthenticationMethod::AuthenticationOk => {}
            AuthenticationMethod::AuthenticationCleartextPassword => {}
            AuthenticationMethod::Md5Password { salt } => {
                method_bytes.put_slice(&salt);
            }
            AuthenticationMethod::Sasl { mechanisms } => {
                for m in mechanisms {
                    let s = m.to_string();
                    let c = CString::new(s)?;
                    let s = c.as_bytes_with_nul();
                    method_bytes.put_slice(s);
                }
                method_bytes.put_u8(0); // null byte
            }
            AuthenticationMethod::AuthenticationSASLContinue { bytes, .. } => {
                method_bytes.put_slice(&bytes);
            }
            AuthenticationMethod::AuthenticationSASLFinal { bytes, .. } => {
                method_bytes.put_slice(&bytes);
            }
            AuthenticationMethod::Other { bytes, .. } => {
                method_bytes.put_slice(&bytes);
            }
        }

        let mut bytes = BytesMut::new();

        let len = SIZE_I32   // message len
                        + method_bytes.len(); // method_code

        bytes.put_u8(BackendCode::Authentication.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(&method_bytes);

        Ok(bytes)
    }
}

impl TryFrom<&BytesMut> for PasswordMessage {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<PasswordMessage, Self::Error> {
        let mut cursor = Cursor::new(bytes);
        let code = cursor.get_u8();

        if FrontendCode::from(code) != FrontendCode::PasswordMessage {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: FrontendCode::PasswordMessage.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // read and progress cursor
        let password = cursor.read_string()?;

        Ok(PasswordMessage { code, password })
    }
}

impl TryFrom<PasswordMessage> for BytesMut {
    type Error = Error;

    fn try_from(password_message: PasswordMessage) -> Result<BytesMut, Self::Error> {
        let mut bytes = BytesMut::new();

        let password = CString::new(password_message.password)?;
        let password = password.as_bytes_with_nul();

        let len = SIZE_I32         // message len
                        + password.len(); // password

        bytes.put_u8(FrontendCode::PasswordMessage.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(password);

        Ok(bytes)
    }
}

impl TryFrom<String> for SaslMechanism {
    type Error = Error;
    fn try_from(s: String) -> Result<SaslMechanism, Self::Error> {
        match s.as_str() {
            SCRAM_SHA_256 => Ok(SaslMechanism::ScramSha256),
            SCRAM_SHA_256_PLUS => Ok(SaslMechanism::ScramSha256Plus),
            s => Err(ProtocolError::UnexpectedSaslAuthenticationMethod(s.to_owned()).into()),
        }
    }
}

impl Display for SaslMechanism {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            SaslMechanism::ScramSha256 => SCRAM_SHA_256.to_owned(),
            SaslMechanism::ScramSha256Plus => SCRAM_SHA_256_PLUS.to_owned(),
        };
        write!(f, "{}", s)
    }
}

impl From<&AuthenticationMethod> for i32 {
    fn from(method: &AuthenticationMethod) -> Self {
        match method {
            AuthenticationMethod::AuthenticationOk => 0,
            AuthenticationMethod::AuthenticationCleartextPassword => 3,
            AuthenticationMethod::Md5Password { .. } => 5,
            AuthenticationMethod::Sasl { .. } => 10,
            AuthenticationMethod::AuthenticationSASLContinue { .. } => 11,
            AuthenticationMethod::AuthenticationSASLFinal { .. } => 12,
            AuthenticationMethod::Other { method_code, .. } => *method_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use crate::{config::LogConfig, log};

    use super::Authentication;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn parse_auth_message() {
        log::init(LogConfig::default());

        let bytes = to_message(b"R\0\0\0*\0\0\0\nSCRAM-SHA-256-PLUS\0SCRAM-SHA-256\0\0");

        let auth = Authentication::try_from(&bytes).unwrap();

        assert!(matches!(
            auth.method,
            super::AuthenticationMethod::Sasl { .. }
        ));

        let auth_bytes = BytesMut::try_from(auth).unwrap();

        assert_eq!(bytes, auth_bytes);
    }

    #[test]
    pub fn is_scram_sha_256_plus() {
        log::init(LogConfig::default());

        let bytes = to_message(b"R\0\0\0*\0\0\0\nSCRAM-SHA-256-PLUS\0SCRAM-SHA-256\0\0");
        let auth = Authentication::try_from(&bytes).unwrap();

        assert!(auth.is_scram_sha_256_plus());
    }
}
