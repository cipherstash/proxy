mod backend;
mod frontend;

use super::Context;
use crate::{error::Error, proxy::EncryptionService};
use backend::Backend;
use frontend::Frontend;
use pg_proto::{
    BackendBatchOutput, BackendFlushReason, BackendMessage, BackendMiddlewareOutput,
    FrontendMessage, FrontendMiddlewareOutput, HeldBackendMessages, IntermediaryMiddleware,
    IntermediaryMiddlewareFactory,
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
{
    type Error = Error;

    async fn frontend(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        message: FrontendMessage,
    ) -> Result<FrontendMiddlewareOutput, Error> {
        self.frontend.intercept(message).await
    }

    async fn backend(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        self.backend.intercept(message).await
    }

    async fn flush_backend(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        held: HeldBackendMessages<'_>,
        _: BackendFlushReason,
    ) -> Result<BackendBatchOutput, Error> {
        self.backend.flush_held(held).await
    }
}
