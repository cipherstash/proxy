use crate::{expected_message, postgresql::mitm::{
    messages::client::{Parse, Query},
    SessionState,
}};

pub(super) struct Ready;

expected_message!(Ready => Query | Parse);

impl SessionState for Ready {
    type ExpectedMessage = ReadyNextMessage;
}
