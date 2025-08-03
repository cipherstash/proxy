pub(super) mod client;
pub(super) mod server;

#[macro_export]
macro_rules! expected_message {
    ($state:ident => $message_a:ident | $message_b:ident) => {
        paste::paste! {
            pub(crate) enum [<$state NextMessage>] {
                $message_a($message_a),
                $message_b($message_b),
            }

            impl core::convert::TryFrom<(char, bytes::BytesMut)> for [<$state NextMessage>] {
                type Error = crate::error::Error;

                fn try_from(value: (char, bytes::BytesMut)) -> Result<Self, Self::Error> {
                    if $message_a::code() == value.0 {
                        return Ok(Self::$message_a($message_a(value.1)))
                    }

                    if $message_b::code() == value.0 {
                        return Ok(Self::$message_b($message_b(value.1)))
                    }

                    Err(crate::error::ProtocolError::UnexpectedMessageCode {
                        expected: vec![$message_a::code(), $message_b::code()],
                        received: value.0,
                    }.into())
                }
            }
        }
    };

    ($state:ident => $message_a:ident, $message_b:ident | $message_c:ident) => {
        paste::paste! {
            pub(crate) enum [<$state NextMessage>] {
                $message_a($message_a),
                $message_b($message_b),
                $message_c($message_c),
            }

            impl core::convert::TryFrom<(char, bytes::BytesMut)> for [<$state NextMessage>] {
                type Error = crate::error::Error;

                fn try_from(value: (char, bytes::BytesMut)) -> Result<Self, Self::Error> {
                    if $message_a::code() == value.0 {
                        return Ok(Self::$message_a($message_a(value.1)))
                    }

                    if $message_b::code() == value.0 {
                        return Ok(Self::$message_b($message_b(value.1)))
                    }

                    if $message_c::code() == value.0 {
                        return Ok(Self::$message_c($message_c(value.1)))
                    }

                    Err(crate::error::ProtocolError::UnexpectedMessageCode {
                        expected: vec![$message_a::code(), $message_b::code(), $message_c::code()],
                        received: value.0,
                    }.into())
                }
            }
        }
    };
}

#[macro_export]
macro_rules! client_message {
    ($name:ident $code:literal) => {
        pub(crate) struct $name(pub(crate) bytes::BytesMut);

        impl $name {
            pub(crate) const fn code() -> char {
                $code
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
    }
}
