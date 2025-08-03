use crate::{error::Error, server_message};

server_message!(Authentication 'R');
server_message!(BackendKeyData 'K');
server_message!(BindComplete '2');
server_message!(CloseComplete '3');
server_message!(CommandComplete 'C');
server_message!(CopyBothResponse 'W');
server_message!(CopyInResponse 'G');
server_message!(CopyOutResponse 'H');
server_message!(DataRow 'D');
server_message!(EmptyQueryResponse 'I');
server_message!(ErrorResponse 'E');
server_message!(NoData 'n');
server_message!(NoticeResponse 'N');
server_message!(NotificationResponse 'A');
server_message!(ParameterDescription 't');
server_message!(ParameterStatus 'S');
server_message!(ParseComplete '1');
server_message!(PortalSuspended 's');
server_message!(ReadyForQuery 'Z');
server_message!(RowDescription 'T');

pub trait ServerMessageMapper {
    fn map_authentication(&self, message: Authentication) -> Result<Authentication, Error>;
    fn map_backend_key_data(&self, message: BackendKeyData) -> Result<BackendKeyData, Error>;
    fn map_bind_complete(&self, message: BindComplete) -> Result<BindComplete, Error>;
    fn map_close_complete(&self, message: CloseComplete) -> Result<CloseComplete, Error>;
    fn map_command_complete(&self, message: CommandComplete) -> Result<CommandComplete, Error>;
    fn map_copy_both_response(&self, message: CopyBothResponse) -> Result<CopyBothResponse, Error>;
    fn map_copy_in_response(&self, message: CopyInResponse) -> Result<CopyInResponse, Error>;
    fn map_copy_out_response(&self, message: CopyOutResponse) -> Result<CopyOutResponse, Error>;
    fn map_data_row(&self, message: DataRow) -> Result<DataRow, Error>;
    fn map_empty_query_response(
        &self,
        message: EmptyQueryResponse,
    ) -> Result<EmptyQueryResponse, Error>;
    fn map_error_response(&self, message: ErrorResponse) -> Result<ErrorResponse, Error>;
    fn map_no_data(&self, message: NoData) -> Result<NoData, Error>;
    fn map_notice_response(&self, message: NoticeResponse) -> Result<NoticeResponse, Error>;
    fn map_notification_response(
        &self,
        message: NotificationResponse,
    ) -> Result<NotificationResponse, Error>;
    fn map_parameter_description(
        &self,
        message: ParameterDescription,
    ) -> Result<ParameterDescription, Error>;
    fn map_parameter_status(&self, message: ParameterStatus) -> Result<ParameterStatus, Error>;
    fn map_parse_complete(&self, message: ParseComplete) -> Result<ParseComplete, Error>;
    fn map_portal_suspended(&self, message: PortalSuspended) -> Result<PortalSuspended, Error>;
    fn map_ready_for_query(&self, message: ReadyForQuery) -> Result<ReadyForQuery, Error>;
    fn map_row_description(&self, message: RowDescription) -> Result<RowDescription, Error>;
}

pub struct ServerMessageNoopMapper;

pub static NOOP_SERVER_MSG_MAPPER: ServerMessageNoopMapper = ServerMessageNoopMapper;

impl ServerMessageMapper for ServerMessageNoopMapper {
    fn map_authentication(&self, message: Authentication) -> Result<Authentication, Error> {
        Ok(message)
    }

    fn map_backend_key_data(&self, message: BackendKeyData) -> Result<BackendKeyData, Error> {
        Ok(message)
    }

    fn map_bind_complete(&self, message: BindComplete) -> Result<BindComplete, Error> {
        Ok(message)
    }

    fn map_close_complete(&self, message: CloseComplete) -> Result<CloseComplete, Error> {
        Ok(message)
    }

    fn map_command_complete(&self, message: CommandComplete) -> Result<CommandComplete, Error> {
        Ok(message)
    }

    fn map_copy_both_response(&self, message: CopyBothResponse) -> Result<CopyBothResponse, Error> {
        Ok(message)
    }

    fn map_copy_in_response(&self, message: CopyInResponse) -> Result<CopyInResponse, Error> {
        Ok(message)
    }

    fn map_copy_out_response(&self, message: CopyOutResponse) -> Result<CopyOutResponse, Error> {
        Ok(message)
    }

    fn map_data_row(&self, message: DataRow) -> Result<DataRow, Error> {
        Ok(message)
    }

    fn map_empty_query_response(
        &self,
        message: EmptyQueryResponse,
    ) -> Result<EmptyQueryResponse, Error> {
        Ok(message)
    }

    fn map_error_response(&self, message: ErrorResponse) -> Result<ErrorResponse, Error> {
        Ok(message)
    }

    fn map_no_data(&self, message: NoData) -> Result<NoData, Error> {
        Ok(message)
    }

    fn map_notice_response(&self, message: NoticeResponse) -> Result<NoticeResponse, Error> {
        Ok(message)
    }

    fn map_notification_response(
        &self,
        message: NotificationResponse,
    ) -> Result<NotificationResponse, Error> {
        Ok(message)
    }

    fn map_parameter_description(
        &self,
        message: ParameterDescription,
    ) -> Result<ParameterDescription, Error> {
        Ok(message)
    }

    fn map_parameter_status(&self, message: ParameterStatus) -> Result<ParameterStatus, Error> {
        Ok(message)
    }

    fn map_parse_complete(&self, message: ParseComplete) -> Result<ParseComplete, Error> {
        Ok(message)
    }

    fn map_portal_suspended(&self, message: PortalSuspended) -> Result<PortalSuspended, Error> {
        Ok(message)
    }

    fn map_ready_for_query(&self, message: ReadyForQuery) -> Result<ReadyForQuery, Error> {
        Ok(message)
    }

    fn map_row_description(&self, message: RowDescription) -> Result<RowDescription, Error> {
        Ok(message)
    }
}
