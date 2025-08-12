
use crate::{error::Error, postgresql::mitm::{
    messages::{client::{self}, server::{self}}, Session, SessionState
}};

pub(crate) trait Transition<FromState, OnMessage, ToState> {
    type ToState;

    async fn transition(self, message: OnMessage) -> Result<Self::ToState, crate::error::Error>;
}

#[macro_export]
macro_rules! transitions {
    (state($state:ident) => $message:ident) => {};
}

#[macro_export]
macro_rules! state {
    ($state:ident) => {
        #[derive(Debug)]
        pub(crate) struct $state;
    }
}

pub(crate) mod state {
    pub(crate) mod authenticated {
        state!(Authenticated);
        state!(ReceivedBackendKeyData);
        state!(ReceivedParameterStatus);
    }

    state!(Ready);

    pub(crate) mod simple {
        state!(SentQuery);
        state!(ReceivedRowDescription);
        state!(ReceivedDataRow);
        state!(ReceivedCommandComplete);
    }

    pub(crate) mod extended {
        state!(SentParse);
        state!(SentBind);
        state!(SentDescribe);
        state!(SentExecute);
        state!(SentSync);
        state!(ReceivedRowDescription);
        state!(ReceivedDataRow);
        state!(ReceivedCommandComplete);
    }

    state!(Terminated);
}

pub(crate) trait OnMessage<Message> {
    type NextState;

    async fn on_message(self, message: Message) -> Result<Self::NextState, Error>;
}

impl<CMM, SMM> OnMessage<server::BackendKeyData> for Session<state::authenticated::Authenticated, CMM, SMM> {
    type NextState = Session<state::authenticated::ReceivedBackendKeyData, CMM, SMM>;

    async fn on_message(self, _message: server::BackendKeyData) -> Result<Self::NextState, Error> {
        todo!()
    }
}

impl<CMM, SMM> OnMessage<server::ParameterStatus> for Session<state::authenticated::ReceivedBackendKeyData, CMM, SMM> {
    type NextState = Session<state::authenticated::ReceivedParameterStatus, CMM, SMM>;

    async fn on_message(self, _message: server::ParameterStatus) -> Result<Self::NextState, Error> {
        todo!()
    }
}

#[macro_export]
macro_rules! transition {
    ()
}


transitions!(
    from(state::authenticated::Authenticated) {
        on_message(server::BackendKeyData) => to(state::authenticated::ReceivedBackendKeyData)
    }

    from(state::authenticated::ReceivedBackendKeyData) {
        on_message(server::ParameterStatus) => to(state::authenticated::ReceivedParameterStatus)
    }

    from(state::authenticated::ReceivedParameterStatus) {
        on_message(server::ReadyForQuery) => state(state::Ready)
    }

    from(state::authenticated::Ready) {
        on_message(client::Query) => to(state::simple::SentQuery) // to simple protcol
        on_message(client::Parse) => to(state::extended::SentParse) // to extended protocol
    }

    from(state::simple::SentQuery) {
        on_message(server::RowDescription) => to(state::simple::ReceivedRowDescription)
        on_message(server::CommandComplete) => to(state::simple::ReceivedCommandComplete)
    }

    from(state::simple::ReceivedRowDescription) {
        on_message(server::DataRow) => to(state::simple::ReceivedDataRow)
    }

    from(state::simple::ReceivedDataRow) {
        on_message(server::DataRow) => to(Self)
        on_message(server::CommandComplete) => to(state::simple::ReceivedCommandComplete)
    }

    from(state::simple::ReceivedCommandComplete) {
        on_message(server::DataRow) => to(Self)
        on_message(server::ReadyForQuery) => state(state::Ready)
    }

    from(state::extended::SentParse) {
        on_message(client::Bind) -> to(state::extended::SentBind),
    }

    from(state::extended::SentBind) {
        on_message(client::Describe) -> to(state::extended::SentDescribe),
        on_message(client::Execute) -> to(state::extended::SentExecute),
    }

    from(state::extended::SentDescribe) {
        on_message(client::Execute) -> to(state::extended::SentExecute),
    }

    from(state::extended::SentExecute) {
        on_message(client::Bind) -> to(state::extended::SentBind),
        on_message(client::Parse) -> to(state::extended::SentParse),
        on_message(client::Sync) -> to(state::extended::SentSync),
    }

    from(state::extended::SentSync) {
        on_message(backend::ReadyForQueryBody) -> to(state::extended::SentSync),
    }

    from(extended::*) {
        // Pipelining means the following messages can be received in *any* extended protocol state.
        on_message(server::RowDescription) => to(Self)
        on_message(server::ParseComplete) => to(Self)
        on_message(server::BindComplete) => to(Self)
        on_message(server::DataRow) => to(Self)
        on_message(server::CommandComplete) => to(Self)

        // error responses and termination can occur in any extended protocol state
        on_message(server::ErrorResponse) => to(Self)
        on_message(client::Terminate) => to(state::Terminated)
    }

    from(*) {
        // error responses and termination can occur in any top level protocol state
        on_message(server::ErrorResponse) => to(Self)
        on_message(client::Terminate) => to(state::Terminated)
    }
);

impl SessionState for Ready {
    type ExpectedMessage = ReadyNextMessage;
}

impl SessionState for SentQuery {
    type ExpectedMessage = SentQueryNextMessage;
}

impl SessionState for ReceivedRowDescription {
    type ExpectedMessage = ReceivedRowDescriptionNextMessage;
}

impl SessionState for ReceivedDataRow {
    type ExpectedMessage = ReceivedDataRowNextMessage;
}

impl SessionState for ReceivedCommandComplete {
    type ExpectedMessage = ReceivedRowDescriptionNextMessage;
}
