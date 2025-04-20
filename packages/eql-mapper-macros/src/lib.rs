use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse::Parse, parse_macro_input, parse_quote, Attribute, FnArg, Ident, ImplItem,
    ImplItemFn, ItemImpl, Pat, PatType, Signature, Type, TypePath, TypeReference,
};

/// This macro generates consistently defined `#[tracing::instrument]` attributes for `InferType::infer_enter` &
/// `InferType::infer_enter` implementations on `TypeInferencer`.
///
/// This attribute MUST be defined on the trait `impl` itself (not the trait method impls).
#[proc_macro_attribute]
pub fn trace_infer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemImpl);

    for item in &mut input.items {
        if let ImplItem::Fn(ImplItemFn {
            attrs,
            sig:
                Signature {
                    ident: method,
                    inputs,
                    ..
                },
            ..
        }) = item
        {
            let node_ident_and_type: Option<(&Ident, &Type)> =
                if let Some(FnArg::Typed(PatType {
                    ty: node_ty, pat, ..
                })) = inputs.get(1)
                {
                    if let Pat::Ident(pat_ident) = &**pat {
                        Some((&pat_ident.ident, &node_ty))
                    } else {
                        None
                    }
                } else {
                    None
                };

            let vec_ident: Ident = parse_quote!(Vec);

            match node_ident_and_type {
                Some((node_ident, node_ty)) => {
                    let (formatter, node_ty_abbrev) = match node_ty {
                        Type::Reference(TypeReference { elem, .. }) => match &**elem {
                            Type::Path(TypePath { path, .. }) => {
                                let last_segment = path.segments.last().unwrap();
                                let last_segment_ident = &last_segment.ident;
                                let last_segment_arguments = if last_segment.arguments.is_empty() {
                                    None
                                }  else {
                                    let args = &last_segment.arguments;
                                    Some(quote!(<#args>))
                                };
                                match last_segment_ident {
                                    ident if ident.to_string() == vec_ident.to_string() => {
                                        (quote!(crate::FmtAstVec), quote!(#last_segment_ident #last_segment_arguments))
                                    }
                                    _ => (quote!(crate::FmtAst), quote!(#last_segment_ident #last_segment_arguments))
                                }
                            },
                            _ => unreachable!("Infer::infer_enter/infer_exit has sig: infer_..(&mut self, delete: &'ast N) -> Result<(), TypeError>")
                        },
                            _ => unreachable!("Infer::infer_enter/infer_exit has sig: infer_..(&mut self, delete: &'ast N) -> Result<(), TypeError>")
                    };

                    let node_ty_abbrev = node_ty_abbrev
                        .to_token_stream()
                        .to_string()
                        .replace(" ", "");

                    let target = format!("eql-mapper::{}", method).to_uppercase();

                    let attr: TracingInstrumentAttr = syn::parse2(quote!{
                        #[tracing::instrument(
                            target = #target,
                            level = "trace",
                            skip(self, #node_ident),
                            fields(
                                ast_ty = #node_ty_abbrev,
                                ast = %#formatter(#node_ident),
                                inferred = self.peek_node_type(#node_ident).map(|n| n.to_string()).unwrap_or("<no-type>".to_owned())
                            ),
                            ret(Debug)
                        )]
                    }).unwrap();
                    attrs.push(attr.attr);
                }
                None => {
                    return quote!(compile_error!(
                        "could not determine name of node argumemt in Infer impl"
                    ))
                    .to_token_stream()
                    .into();
                }
            }
        }
    }

    input.to_token_stream().into()
}

struct TracingInstrumentAttr {
    attr: Attribute,
}

impl Parse for TracingInstrumentAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            attr: Attribute::parse_outer(input)?.get(0).unwrap().clone(),
        })
    }
}
