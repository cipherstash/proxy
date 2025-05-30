mod trace_infer;
use trace_infer::*;
mod parse;

use proc_macro::TokenStream;

/// This macro generates consistently defined `#[tracing::instrument]` attributes for `InferType::infer_enter` &
/// `InferType::infer_enter` implementations on `TypeInferencer`.
///
/// This attribute MUST be defined on the trait `impl` itself (not the trait method impls).
#[proc_macro_attribute]
pub fn trace_infer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    trace_infer_(_attr, item)
}
