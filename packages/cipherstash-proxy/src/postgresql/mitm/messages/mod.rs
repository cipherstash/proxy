pub(super) mod client;
pub(super) mod server;


#[macro_export]
macro_rules! client_message {
    ($name:ident $code:literal) => {
        pub(crate) struct $name(pub(crate) bytes::BytesMut);

        impl $name {
            pub(crate) const fn code() -> char {
                $code
            }
        }

        impl core::convert::TryFrom<(char, bytes::BytesMut)> for $name {
            type Error = crate::error::Error;

            fn try_from(value: (char, bytes::BytesMut)) -> Result<Self, Self::Error> {
                if Self::code() == value.0 {
                    return Ok(Self(value.1))
                }

                Err(crate::error::ProtocolError::UnexpectedMessageCode {
                    expected: vec![Self::code()],
                    received: value.0,
                }.into())
            }
        }
    }
}

#[macro_export]
macro_rules! server_message {
    ($name:ident $code:literal) => {
        pub(crate) struct $name(pub(crate) bytes::BytesMut);

        impl $name {
            pub(crate) const fn code() -> char {
                $code
            }
        }

        impl core::convert::TryFrom<(char, bytes::BytesMut)> for $name {
            type Error = crate::error::Error;

            fn try_from(value: (char, bytes::BytesMut)) -> Result<Self, Self::Error> {
                if Self::code() == value.0 {
                    return Ok(Self(value.1))
                }

                Err(crate::error::ProtocolError::UnexpectedMessageCode {
                    expected: vec![Self::code()],
                    received: value.0,
                }.into())
            }
        }
    }
}
