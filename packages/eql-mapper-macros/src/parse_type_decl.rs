use proc_macro2::token_stream::TokenStream;
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{
    braced, bracketed, parenthesized,
    parse::{ Parse, ParseStream},
    punctuated::Punctuated,
    token::{self},
    Ident, Token, TypePath,
};

mod kw {
    syn::custom_keyword!(Accessor);
    syn::custom_keyword!(Contain);
    syn::custom_keyword!(EQL);
    syn::custom_keyword!(Eq);
    syn::custom_keyword!(Full);
    syn::custom_keyword!(JsonLike);
    syn::custom_keyword!(Native);
    syn::custom_keyword!(Only);
    syn::custom_keyword!(Ord);
    syn::custom_keyword!(Partial);
    syn::custom_keyword!(Path);
    syn::custom_keyword!(SetOf);
    syn::custom_keyword!(TokenMatch);
}

/// Generates a newtype wrapper struct around a `TokenStream` and a implements `ToTokens` for it.
/// The newtype wrapper allows a `syn::parse::Parse` implementation to be attached to it.
macro_rules! tokens_of {
    ($ident:ident) => {
        pub(super) struct $ident(TokenStream);

        impl ToTokens for $ident {
            fn to_tokens(&self, tokens: &mut TokenStream) {
                self.0.to_tokens(tokens);
            }
        }
    };
}

tokens_of!(ArrayDecl);
tokens_of!(AssociatedTypeDecl);
tokens_of!(BinaryOpDecl);
tokens_of!(BoundsDecl);
tokens_of!(EqlTerm);
tokens_of!(EqlTrait);
tokens_of!(EqlTraits);
tokens_of!(FunctionDecl);
tokens_of!(NativeDecl);
tokens_of!(ProjectionColumnDecl);
tokens_of!(ProjectionDecl);
tokens_of!(SetOfDecl);
tokens_of!(SqltkBinOp);
tokens_of!(TVar);
tokens_of!(TableColumn);
tokens_of!(TypeEquation);
tokens_of!(TypeEnvDecl);
tokens_of!(TypeDecl);
tokens_of!(VarDecl);

impl Parse for TVar {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = Ident::parse(input)?.to_string();
        Ok(Self(quote! {
            crate::inference::unifier::TVar(#ident.to_string())
        }))
    }
}

impl Parse for VarDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = Ident::parse(input)?.to_string();
        if input.peek(Token![:]) {
            let _: Token![:] = input.parse()?;
            let bounds = EqlTraits::parse(input)?;
            Ok(Self(quote! {
                crate::inference::unifier::VarDecl {
                    tvar: crate::inference::unifier::TVar(#ident.to_string()),
                    bounds: #bounds,
                }
            }))
        } else {
            Ok(Self(quote! {
                crate::inference::unifier::VarDecl {
                    tvar: crate::inference::unifier::TVar(#ident.to_string()),
                    bounds: crate::inference::unifier::EqlTraits::default(),
                }
            }))
        }
    }
}

impl Parse for EqlTraits {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut traits: Vec<EqlTrait> = Vec::new();

        loop {
            traits.push(EqlTrait::parse(input)?);

            if !input.peek(token::Plus) {
                break;
            }

            token::Plus::parse(input)?;
        }

        Ok(Self(quote!(
            crate::inference::unifier::EqlTraits::from_iter(vec![#(#traits),*])
        )))
    }
}

impl Parse for BoundsDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut traits: Vec<EqlTrait> = Vec::new();

        let tvar = TVar::parse(input)?;

        let _: token::Colon = input.parse()?;

        loop {
            traits.push(EqlTrait::parse(input)?);

            if !input.peek(token::Plus) {
                break;
            }

            let _: token::Plus = input.parse()?;
        }

        Ok(Self(quote! {
            crate::inference::unifier::BoundsDecl(
                #tvar,
                crate::inference::unifier::EqlTraits::from_iter(vec![#(#traits),*])
            )
        }))
    }
}

impl Parse for EqlTrait {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(kw::Eq) {
            kw::Eq::parse(input)?;
            return Ok(Self(quote!(crate::inference::unifier::EqlTrait::Eq)));
        }

        if input.peek(kw::Ord) {
            kw::Ord::parse(input)?;
            return Ok(Self(quote!(crate::inference::unifier::EqlTrait::Ord)));
        }

        if input.peek(kw::TokenMatch) {
            kw::TokenMatch::parse(input)?;
            return Ok(Self(quote!(
                crate::inference::unifier::EqlTrait::TokenMatch
            )));
        }

        if input.peek(kw::JsonLike) {
            kw::JsonLike::parse(input)?;
            return Ok(Self(quote!(crate::inference::unifier::EqlTrait::JsonLike)));
        }

        if input.peek(kw::Contain) {
            kw::Contain::parse(input)?;
            return Ok(Self(quote!(crate::inference::unifier::EqlTrait::Contain)));
        }

        Err(syn::Error::new(
            input.span(),
            format!(
                "Expected Eq, Ord, TokenMatch or JsonLike while parsing EqlTrait; got: {}",
                input.cursor().token_stream()
            ),
        ))
    }
}

impl Parse for AssociatedTypeDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: token::Lt = input.parse()?;
        let impl_tvar = TVar::parse(input)?;
        let _: token::As = input.parse()?;
        let as_eql_trait = EqlTrait::parse(input)?;
        let _: token::Gt = input.parse()?;
        let _: token::PathSep = input.parse()?;
        let type_name_ident = input.parse::<Ident>()?;
        let type_name = type_name_ident.to_string();

        Ok(Self(quote! {
            crate::inference::unifier::AssociatedTypeDecl {
                impl_decl: Box::new(crate::inference::unifier::TypeDecl::Var(
                    crate::inference::unifier::VarDecl{
                        tvar: #impl_tvar,
                        bounds: crate::inference::unifier::EqlTraits::none()
                    }
                )),
                as_eql_trait: #as_eql_trait,
                type_name: #type_name,
            }
        }))
    }
}

impl Parse for NativeDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: kw::Native = input.parse()?;
        if input.peek(token::Paren) {
            let content;
            parenthesized!(content in input);
            let table_column = TableColumn::parse(&content)?;

            Ok(Self(
                quote!(crate::inference::unifier::NativeDecl(Some(#table_column))),
            ))
        } else {
            Ok(Self(quote!(crate::inference::unifier::NativeDecl(None))))
        }
    }
}

impl Parse for ProjectionColumnDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let spec = TypeDecl::parse(input)?;
        if input.peek(token::As) {
            let _: token::As = input.parse()?;
            let alias = Ident::parse(input)?;
            let alias = alias.to_string();
            Ok(Self(
                quote!(crate::inference::unifier::ProjectionColumnDecl(Box::new(#spec), Some(#alias.into()))),
            ))
        } else {
            Ok(Self(
                quote!(crate::inference::unifier::ProjectionColumnDecl(Box::new(#spec), None)),
            ))
        }
    }
}

impl Parse for ProjectionDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        braced!(content in input);

        let mut specs: Vec<ProjectionColumnDecl> = Vec::new();

        loop {
            specs.push(ProjectionColumnDecl::parse(&content)?);

            if !content.peek(token::Comma) {
                break;
            }

            token::Comma::parse(&content)?;
        }

        Ok(Self(quote!(crate::inference::unifier::ProjectionDecl(
            Vec::from_iter(vec![#(#specs,)*])
        ))))
    }
}

impl Parse for SetOfDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: kw::SetOf = input.parse()?;
        let _: Token![<] = input.parse()?;
        let type_decl = TypeDecl::parse(input)?;
        let _: Token![>] = input.parse()?;

        Ok(Self(
            quote!(crate::inference::unifier::SetOfDecl(Box::new(#type_decl))),
        ))
    }
}

impl Parse for ArrayDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        bracketed!(content in input);

        let type_spec = TypeDecl::parse(&content)?;

        Ok(Self(
            quote!(crate::inference::unifier::ArrayDecl(Box::new(#type_spec))),
        ))
    }
}

impl Parse for EqlTerm {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: kw::EQL = input.parse()?;

        let content;
        parenthesized!(content in input);

        let table = Ident::parse(&content)?;
        let table = table.to_string();
        let _: token::Dot = content.parse()?;
        let column = Ident::parse(&content)?;
        let column = column.to_string();

        if content.peek(token::Colon) {
            let _: token::Colon = content.parse()?;
            let bounds = EqlTraits::parse(&content)?;

            Ok(Self(quote! {
                crate::inference::unifier::EqlTerm::Full(
                    crate::inference::unifier::EqlValue(
                        crate::inference::unifier::TableColumn {
                            table: #table.into(),
                            column: #column.into(),
                        },
                        #bounds,
                    ),
                )
            }))
        } else {
            Ok(Self(quote! {
                crate::inference::unifier::EqlTerm::Full(
                    crate::inference::unifier::EqlValue(
                        crate::inference::unifier::TableColumn {
                            table: #table,
                            column: #column
                        },
                    ),
                    crate::inference::unifier::EqlTraits::none(),
                )
            }))
        }
    }
}

impl Parse for TypeDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if AssociatedTypeDecl::parse(&input.fork()).is_ok() {
            let inner = AssociatedTypeDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::AssociatedType(#inner)
            }));
        }

        if SetOfDecl::parse(&input.fork()).is_ok() {
            let inner = SetOfDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::SetOf(#inner)
            }));
        }

        if NativeDecl::parse(&input.fork()).is_ok() {
            let inner = NativeDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::Native(#inner)
            }));
        }

        if EqlTerm::parse(&input.fork()).is_ok() {
            let inner = EqlTerm::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::Eql(#inner)
            }));
        }

        if VarDecl::parse(&input.fork()).is_ok() {
            let inner = VarDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::Var(#inner)
            }));
        }

        if ArrayDecl::parse(&input.fork()).is_ok() {
            let inner = ArrayDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::Array(#inner)
            }));
        }

        if ProjectionDecl::parse(&input.fork()).is_ok() {
            let inner = ProjectionDecl::parse(input)?;
            return Ok(Self(quote! {
                crate::inference::unifier::TypeDecl::Projection(#inner)
            }));
        }

        Err(syn::Error::new(
            input.span(),
            "could not parse as TypeDecl".to_string(),
        ))
    }
}

impl Parse for FunctionDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let schema = Ident::parse(input)?;
        let schema = schema.to_string();
        let _: token::Dot = input.parse()?;
        let function_name = Ident::parse(input)?;
        let function_name = function_name.to_string();

        let generic_args = if input.peek(token::Lt) {
            token::Lt::parse(input)?;
            let args = Punctuated::<TVar, Token![,]>::parse_separated_nonempty(input)?
                .into_iter()
                .collect();
            token::Gt::parse(input)?;
            args
        } else {
            Vec::new()
        };

        let content;
        parenthesized!(content in input);

        let args: Vec<TypeDecl> =
            Punctuated::<TypeDecl, Token![,]>::parse_separated_nonempty(&content)?
                .into_iter()
                .collect();

        let _: token::RArrow = input.parse()?;

        let ret = TypeDecl::parse(input)?;

        let bounds: Vec<_> = if input.peek(token::Where) {
            let _: token::Where = input.parse()?;
            let boundeds = Punctuated::<BoundsDecl, Token![,]>::parse_separated_nonempty(input)?;
            boundeds.into_iter().collect()
        } else {
            vec![]
        };

        Ok(Self(quote! {
            crate::inference::unifier::FunctionDecl {
                name: sqltk::parser::ast::ObjectName(vec![
                    sqltk::parser::ast::ObjectNamePart::Identifier(sqltk::parser::ast::Ident::new(#schema)),
                    sqltk::parser::ast::ObjectNamePart::Identifier(sqltk::parser::ast::Ident::new(#function_name)),
                ]),
                inner: crate::inference::unifier::FunctionSignatureDecl::new(
                    vec![#(#generic_args),*],
                    vec![#(#bounds),*],
                    vec![#(#args),*],
                    #ret,
                ).expect("FunctionSignatureDecl creation failed due to a type error"),
            }
        }))
    }
}

impl Parse for TableColumn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let table = Ident::parse(input)?;
        let table = table.to_string();
        let _: token::Dot = input.parse()?;
        let column = Ident::parse(input)?;
        let column = column.to_string();

        Ok(Self(quote! {
            crate::TableColumn {
                table: sqltk::parser::ast::Ident::new(#table),
                column: sqltk::parser::ast::Ident::new(#column),
            }
        }))
    }
}

impl Parse for SqltkBinOp {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(token::RArrow) {
            let _: token::RArrow = input.parse()?;
            if input.peek(token::Gt) {
                let _: token::Gt = input.parse()?;
                return Ok(Self(quote!(
                    ::sqltk::parser::ast::BinaryOperator::LongArrow
                )));
            } else {
                return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::Arrow)));
            }
        }

        if input.peek(token::At) {
            let _: token::At = input.parse()?;
            let _: token::Gt = input.parse()?;
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::AtArrow)));
        }

        if input.peek(token::Le) {
            let _: token::Le = input.parse()?;
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::LtEq)));
        }

        if input.peek(token::Lt) {
            let _: token::Lt = input.parse()?;
            if input.peek(token::At) {
                let _: token::At = input.parse()?;
                return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::ArrowAt)));
            } else if input.peek(token::Gt) {
                let _: token::Gt = input.parse()?;
                return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::NotEq)));
            }
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::Lt)));
        }

        if input.peek(token::Ge) {
            let _: token::Ge = input.parse()?;
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::GtEq)));
        }

        if input.peek(token::Eq) {
            let _: token::Eq = input.parse()?;
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::Eq)));
        }

        if input.peek(token::Gt) {
            let _: token::Gt = input.parse()?;
            return Ok(Self(quote!(::sqltk::parser::ast::BinaryOperator::Gt)));
        }

        if input.peek(token::Tilde) {
            let _: token::Tilde = input.parse()?;
            if input.peek(token::Tilde) {
                let _: token::Tilde = input.parse()?;
                if input.peek(token::Star) {
                    let _: token::Star = input.parse()?;
                    return Ok(Self(quote!(
                        ::sqltk::parser::ast::BinaryOperator::PGILikeMatch
                    )));
                } else {
                    return Ok(Self(quote!(
                        ::sqltk::parser::ast::BinaryOperator::PGLikeMatch
                    )));
                }
            }
        }

        if input.peek(token::Not) {
            let _: token::Not = input.parse()?;
            if input.peek(token::Tilde) {
                let _: token::Tilde = input.parse()?;
                if input.peek(token::Tilde) {
                    let _: token::Tilde = input.parse()?;
                    if input.peek(token::Star) {
                        let _: token::Star = input.parse()?;
                        return Ok(Self(quote!(
                            ::sqltk::parser::ast::BinaryOperator::PGNotILikeMatch
                        )));
                    } else {
                        return Ok(Self(quote!(
                            ::sqltk::parser::ast::BinaryOperator::PGNotLikeMatch
                        )));
                    }
                }
            }
        }

        Err(syn::Error::new(
            input.span(),
            "Expected an operator corresponding to one of the EQL traits Eq, Ord, TokenMatch or JsonLike".to_string(),
        ))
    }
}

impl Parse for BinaryOpDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let generic_args = if input.peek(token::Lt) {
            token::Lt::parse(input)?;
            let args = Punctuated::<TVar, Token![,]>::parse_separated_nonempty(input)?
                .into_iter()
                .collect();
            token::Gt::parse(input)?;
            args
        } else {
            Vec::new()
        };

        let content;
        parenthesized!(content in input);
        let lhs = TypeDecl::parse(&content)?;
        let op = SqltkBinOp::parse(&content)?;
        let rhs = TypeDecl::parse(&content)?;

        let _: token::RArrow = input.parse()?;
        let ret = TypeDecl::parse(input)?;

        let bounds: Vec<_> = if input.peek(token::Where) {
            let _: token::Where = input.parse()?;
            let boundeds = Punctuated::<BoundsDecl, Token![,]>::parse_separated_nonempty(input)?;
            boundeds.into_iter().collect()
        } else {
            vec![]
        };

        Ok(Self(quote! {
            crate::inference::unifier::BinaryOpDecl {
                op: #op,
                inner: crate::inference::unifier::FunctionSignatureDecl::new(
                    vec![#(#generic_args),*],
                    vec![#(#bounds),*],
                    vec![#lhs, #rhs],
                    #ret,
                ).expect("FunctionSignatureDecl creation failed due to a type error"),
            }
        }))
    }
}

impl Parse for TypeEquation {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let tvar = TVar::parse(input)?;
        let _ = token::Eq::parse(input)?;
        let type_decl = TypeDecl::parse(input)?;

        Ok(Self(quote! {
            env.add_decl(#tvar, #type_decl);
        }))
    }
}

impl Parse for TypeEnvDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let decls: Vec<_> = Punctuated::<TypeEquation, Token![;]>::parse_terminated(input)?
            .into_iter()
            .collect();

        Ok(Self(quote! {
            {
                let mut env = crate::inference::unifier::TypeEnv::new();
                #( #decls )*
                env
            }
        }))
    }
}

pub(crate) struct BinaryOpDecls {
    ops: Vec<BinaryOpDecl>,
}

impl ToTokens for BinaryOpDecls {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ops = &self.ops;
        tokens.append_all(quote!(vec![#(#ops),*]));
    }
}

impl Parse for BinaryOpDecls {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ops = Punctuated::<BinaryOpDecl, Token![;]>::parse_terminated(input)?
            .into_iter()
            .collect();
        Ok(Self { ops })
    }
}

pub(crate) struct FunctionDecls {
    ops: Vec<FunctionDecl>,
}

impl ToTokens for FunctionDecls {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ops = &self.ops;
        tokens.append_all(quote!(vec![#(#ops),*]));
    }
}

impl Parse for FunctionDecls {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ops = Punctuated::<FunctionDecl, Token![;]>::parse_terminated(input)?
            .into_iter()
            .collect();
        Ok(Self { ops })
    }
}

pub(crate) struct ShallowInitTypes {
    pub(crate) unifier: syn::Expr,
    pub(crate) bindings: Vec<Binding>,
}

impl Parse for ShallowInitTypes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let unifier = syn::Expr::parse(input)?;
        let _: Token![,] = input.parse()?;
        let content;
        braced!(content in input);
        let bindings: Vec<Binding> = Punctuated::<Binding, Token![;]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        Ok(Self { unifier, bindings })
    }
}

impl ToTokens for ShallowInitTypes {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let unifier = &self.unifier;
        for binding in self.bindings.iter() {
            tokens.append_all(quote! {
                #binding.instantiate_shallow(#unifier).unwrap();
            });
        }
    }
}

pub(crate) struct ConcreteTyArgs {
    pub(crate) ty_decl: TypeDecl,
    pub(crate) ty_as: Option<TypePath>,
}

impl Parse for ConcreteTyArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ty_decl = input.parse()?;
        let ty_as = if input.peek(Token![as]) {
            let _: Token![as] = input.parse()?;
            let ty: TypePath = input.parse()?;
            Some(ty)
        } else {
            None
        };

        Ok(Self { ty_decl, ty_as })
    }
}

pub(crate) struct Binding {
    pub(crate) var: Ident,
    pub(crate) type_decl: TypeDecl,
}

impl Parse for Binding {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _ = syn::token::Let::parse(input)?;
        let var = Ident::parse(input)?;
        let _: Token![=] = input.parse()?;
        let type_decl = TypeDecl::parse(input)?;

        Ok(Self { var, type_decl })
    }
}

impl ToTokens for Binding {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let var = &self.var;
        let type_decl = &self.type_decl;

        tokens.append_all(quote! {
            let #var = #type_decl
        });
    }
}

#[cfg(test)]
mod test {
    use quote::quote;
    use syn::parse2;
    use pretty_assertions::assert_eq;

    use crate::parse_type_decl::{AssociatedTypeDecl, BinaryOpDecl, TVar};

    #[test]
    fn parse_tvar() {
        let parsed: TVar = parse2(quote!(T)).unwrap();

        assert_eq!(
            parsed.0.to_string(),
            quote!(crate::inference::unifier::TVar("T".to_string())).to_string()
        );
    }

    #[test]
    fn parse_associated_type() {
        let parsed: AssociatedTypeDecl = parse2(quote!(<T as JsonLike>::Accessor)).unwrap();

        assert_eq!(
            parsed.0.to_string(),
            quote!(crate::inference::unifier::AssociatedTypeDecl {
                impl_decl: Box::new(crate::inference::unifier::TypeDecl::Var(
                    crate::inference::unifier::VarDecl{
                        tvar: crate::inference::unifier::TVar("T".to_string()),
                        bounds: crate::inference::unifier::EqlTraits::none()
                    }
                )),
                as_eql_trait: crate::inference::unifier::EqlTrait::JsonLike,
                type_name: "Accessor",
            })
            .to_string()
        );
    }

    #[test]
    fn parse_binary_operators() {
        let parsed: BinaryOpDecl = parse2(quote!(<T>(T = T) -> Native where T: Eq)).unwrap();

        assert_eq!(
            parsed.0.to_string(),
            quote!(crate::inference::unifier::BinaryOpDecl {
                op: ::sqltk::parser::ast::BinaryOperator::Eq,
                inner: crate::inference::unifier::FunctionSignatureDecl::new(
                    vec![crate::inference::unifier::TVar("T".to_string())],
                    vec![crate::inference::unifier::BoundsDecl(
                        crate::inference::unifier::TVar("T".to_string()),
                        crate::inference::unifier::EqlTraits::from_iter(vec![
                            crate::inference::unifier::EqlTrait::Eq
                        ])
                    )],
                    vec![
                        crate::inference::unifier::TypeDecl::Var(
                            crate::inference::unifier::VarDecl {
                                tvar: crate::inference::unifier::TVar("T".to_string()),
                                bounds: crate::inference::unifier::EqlTraits::default(),
                            }
                        ),
                        crate::inference::unifier::TypeDecl::Var(
                            crate::inference::unifier::VarDecl {
                                tvar: crate::inference::unifier::TVar("T".to_string()),
                                bounds: crate::inference::unifier::EqlTraits::default(),
                            }
                        )
                    ],
                    crate::inference::unifier::TypeDecl::Native(
                        crate::inference::unifier::NativeDecl(None)
                    ),
                )
                .expect("FunctionSignatureDecl creation failed due to a type error"),
            })
            .to_string()
        );
    }
}
