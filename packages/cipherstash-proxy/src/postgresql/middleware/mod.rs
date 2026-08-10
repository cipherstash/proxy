mod backend;
mod frontend;

use crate::{error::Error, proxy::EncryptionService};
use pg_proto::{
    BackendBatchOutput, BackendFlushReason, BackendMessage, BackendMiddlewareOutput,
    FrontendMessage, FrontendMiddlewareOutput, HeldBackendMessages, IntermediaryMiddleware,
    IntermediaryMiddlewareFactory,
};
use std::{future::Future, pin::Pin};

use super::Context;
use backend::Backend;
use frontend::Frontend;

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

    fn frontend<'a>(
        &'a mut self,
        _: &'a ServerContext,
        _: &'a ClientContext,
        _: &'a mut (),
        message: FrontendMessage,
    ) -> Pin<Box<dyn Future<Output = Result<FrontendMiddlewareOutput, Error>> + 'a>> {
        Box::pin(async move { self.frontend.intercept(message).await })
    }

    fn backend<'a>(
        &'a mut self,
        _: &'a ServerContext,
        _: &'a ClientContext,
        _: &'a mut (),
        message: BackendMessage,
    ) -> Pin<Box<dyn Future<Output = Result<BackendMiddlewareOutput, Error>> + 'a>> {
        Box::pin(async move { self.backend.intercept(message).await })
    }

    fn flush_backend<'a>(
        &'a mut self,
        _: &'a ServerContext,
        _: &'a ClientContext,
        _: &'a mut (),
        held: HeldBackendMessages<'a>,
        _: BackendFlushReason,
    ) -> Pin<Box<dyn Future<Output = Result<BackendBatchOutput, Error>> + 'a>> {
        Box::pin(async move { self.backend.flush_held(held).await })
    }
}
