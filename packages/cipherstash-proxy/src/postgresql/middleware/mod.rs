mod backend;
mod frontend;

use super::Context;
use crate::{error::Error, proxy::EncryptionService};
use backend::Backend;
use frontend::Frontend;
use pg_proto::{
    AttributedBackendMessages, BackendBatchOutput, BackendFlushReason, BackendMessage,
    BackendMiddlewareOutput, FrontendMessage, FrontendMiddlewareOutput, IntermediaryMiddleware,
    IntermediaryMiddlewareFactory, OperationId,
};

pub struct CipherStashMiddleware<S: EncryptionService + Clone> {
    frontend: Frontend<S>,
    backend: Backend<S>,
}

#[derive(Clone)]
pub struct CipherStashMiddlewareFactory<S: EncryptionService + Clone>(pub Context<S>);

impl<S, ServerContext, ClientContext> IntermediaryMiddlewareFactory<ServerContext, ClientContext>
    for CipherStashMiddlewareFactory<S>
where
    S: EncryptionService + Clone,
    ServerContext: Sync,
    ClientContext: Sync,
{
    type Handler = CipherStashMiddleware<S>;
    fn create(&self, _: &ServerContext, _: &ClientContext) -> Self::Handler {
        CipherStashMiddleware::new(self.0.clone())
    }
}

impl<S: EncryptionService + Clone> CipherStashMiddleware<S> {
    pub fn new(context: Context<S>) -> Self {
        Self {
            frontend: Frontend::new(context.clone()),
            backend: Backend::new(context),
        }
    }
}

impl<S, ServerContext, ClientContext> IntermediaryMiddleware<(), ServerContext, ClientContext>
    for CipherStashMiddleware<S>
where
    S: EncryptionService + Clone,
    ServerContext: Sync,
    ClientContext: Sync,
{
    type Error = Error;

    async fn frontend_operation(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        operation: OperationId,
        message: FrontendMessage,
    ) -> Result<FrontendMiddlewareOutput, Error> {
        self.frontend.intercept(operation, message).await
    }

    async fn backend_operation(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        operation: Option<OperationId>,
        message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        self.backend.intercept(operation, message).await
    }

    async fn flush_backend_operations(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        held: AttributedBackendMessages<'_>,
        _: BackendFlushReason,
    ) -> Result<BackendBatchOutput, Error> {
        self.backend.flush_held(held).await
    }
}
