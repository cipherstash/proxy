use sqltk::parser::ast::{BinaryOperator, ObjectName};
use syn::{
    braced, bracketed, parenthesized,
    parse::{discouraged::Speculative, Parse, ParseStream},
    punctuated::Punctuated,
    token, Ident, Result, Token,
};

use crate::{
    unifier::{GeneralizedFunctionSpec, ProjectionColumnSpec},
    EqlValue,
};

use super::{
    ArraySpec, AssociatedTypeSpec, BinaryOpSpec, Bounded, EqlTerm, EqlTrait, EqlTraits,
    FunctionSpec, NativeSpec, ProjectionSpec, TVar, TableColumn, TypeSpec, TypeSpecBounds, VarSpec,
};

impl Parse for TypeSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        if AssociatedTypeSpec::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::AssociatedType(AssociatedTypeSpec::parse(input)?));
        }

        if NativeSpec::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::Native(NativeSpec::parse(input)?));
        }

        if EqlTerm::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::Eql(EqlTerm::parse(input)?));
        }

        if VarSpec::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::Var(VarSpec::parse(input)?));
        }

        if ArraySpec::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::Array(ArraySpec::parse(input)?));
        }

        if ProjectionSpec::parse(&input.fork()).is_ok() {
            return Ok(TypeSpec::Projection(ProjectionSpec::parse(input)?));
        }

        Err(syn::Error::new(
            input.span(),
            format!("could not parse as TypeSpec"),
        ))
    }
}

impl Parse for TVar {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = Ident::parse(input)?;
        Ok(TVar(ident.to_string()))
    }
}

impl Parse for VarSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = Ident::parse(input)?;
        if input.peek(Token![where]) {
            let _: Token![where] = input.parse()?;
            let bounds: EqlTraits = input.parse()?;
            Ok(VarSpec {
                tvar: TVar(ident.to_string()),
                bounds,
            })
        } else {
            Ok(VarSpec {
                tvar: TVar(ident.to_string()),
                bounds: EqlTraits::default(),
            })
        }
    }
}

impl Parse for EqlTraits {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut bounds = EqlTraits::none();

        loop {
            bounds.add_mut(EqlTrait::parse(input)?);

            if !input.peek(token::Plus) {
                break;
            }

            token::Plus::parse(input)?;
        }

        Ok(bounds)
    }
}

/// Everything after the `where`.
///
/// where T: Eq, U: Eq + Json
impl Parse for TypeSpecBounds {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut bounds: Vec<Bounded> = vec![];
        loop {
            let bounded: Bounded = Bounded::parse(input)?;
            bounds.push(bounded);

            if !input.peek(token::Comma) {
                break;
            }

            let _: token::Comma = input.parse()?;
        }

        Ok(TypeSpecBounds(bounds))
    }
}

impl Parse for Bounded {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut traits: Vec<EqlTrait> = vec![];
        let tvar = TVar::parse(input)?;
        let _: token::Colon = input.parse()?;

        loop {
            traits.push(EqlTrait::parse(input)?);

            if !input.peek(token::Plus) {
                break;
            }

            let _: token::Plus = input.parse()?;
        }

        Ok(Bounded(tvar, EqlTraits::from_iter(traits)))
    }
}

mod kw {
    syn::custom_keyword!(EQL);
    syn::custom_keyword!(Full);
    syn::custom_keyword!(Partial);
    syn::custom_keyword!(Native);
    syn::custom_keyword!(Eq);
    syn::custom_keyword!(Ord);
    syn::custom_keyword!(Bloom);
    syn::custom_keyword!(AND);
    syn::custom_keyword!(OR);
    syn::custom_keyword!(Json);
    syn::custom_keyword!(Containment);
    syn::custom_keyword!(JsonFieldAccess);
}

impl Parse for EqlTrait {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(kw::Eq) {
            kw::Eq::parse(input)?;
            return Ok(EqlTrait::Eq);
        }

        if input.peek(kw::Ord) {
            kw::Ord::parse(input)?;
            return Ok(EqlTrait::Ord);
        }

        if input.peek(kw::Bloom) {
            kw::Bloom::parse(input)?;
            return Ok(EqlTrait::Bloom);
        }

        if input.peek(kw::Json) {
            kw::Json::parse(input)?;
            return Ok(EqlTrait::Json);
        }

        if input.peek(kw::Containment) {
            kw::Containment::parse(input)?;
            return Ok(EqlTrait::Containment);
        }

        if input.peek(kw::JsonFieldAccess) {
            kw::Containment::parse(input)?;
            return Ok(EqlTrait::JsonFieldAccess);
        }

        Err(syn::Error::new(
            input.span(),
            format!(
                "Expected Eq, Ord, Bloom or Json while parsing EqlTrait; got: {}",
                input.cursor().token_stream()
            ),
        ))
    }
}

impl Parse for AssociatedTypeSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let forked = input.fork();

        if let Ok(tvar) = TVar::parse(&forked) {
            input.advance_to(&forked);
            let _: token::PathSep = input.parse()?;
            let associated_type_ctor = AssociatedTypeSpecCtor::parse(input)?;
            return Ok(associated_type_ctor.0(tvar));
        }

        // if let Ok(native_spec) = NativeSpec::parse(&forked) {
        //     input.advance_to(&forked);
        //     let _: token::PathSep = input.parse()?;
        //     let associated_type_ctor = AssociatedTypeSpecCtor::parse(input)?;
        //     return Ok(associated_type_ctor.0(TypeSpec::Native(native_spec)));
        // }

        // if let Ok(eql_term) = EqlTerm::parse(&forked) {
        //     input.advance_to(&forked);
        //     let _: token::PathSep = input.parse()?;
        //     let associated_type_ctor = AssociatedTypeSpecCtor::parse(input)?;
        //     return Ok(associated_type_ctor.0(TypeSpec::Eql(eql_term)));
        // }

        // if let Ok(array_spec) = ArraySpec::parse(&forked) {
        //     input.advance_to(&forked);
        //     let _: token::PathSep = input.parse()?;
        //     let associated_type_ctor = AssociatedTypeSpecCtor::parse(input)?;
        //     return Ok(associated_type_ctor.0(TypeSpec::Array(array_spec)));
        // }

        // if let Ok(projection_spec) = ProjectionSpec::parse(&forked) {
        //     input.advance_to(&forked);
        //     let _: token::PathSep = input.parse()?;
        //     let associated_type_ctor = AssociatedTypeSpecCtor::parse(input)?;
        //     return Ok(associated_type_ctor.0(TypeSpec::Projection(
        //         projection_spec,
        //     )));
        // }

        Err(syn::Error::new(input.span(), "expected type var"))
    }
}

struct AssociatedTypeSpecCtor(fn(TVar) -> AssociatedTypeSpec);

impl Parse for AssociatedTypeSpecCtor {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(kw::Containment) {
            kw::Containment::parse(input)?;
            return Ok(AssociatedTypeSpecCtor(|tvar| AssociatedTypeSpec {
                parent_tvar: tvar,
                associated_type_name: crate::unifier::EQL_TERM_ASSOCIATED_TYPE__CONTAINMENT,
            }));
        }

        if input.peek(kw::JsonFieldAccess) {
            kw::JsonFieldAccess::parse(input)?;
            return Ok(AssociatedTypeSpecCtor(|tvar| AssociatedTypeSpec {
                parent_tvar: tvar,
                associated_type_name: crate::unifier::EQL_TERM_ASSOCIATED_TYPE__JSON_FIELD_ACCESS,
            }));
        }

        return Err(syn::Error::new(
            input.span(),
            "expected associated type Containment or JsonFieldAccess",
        ));
    }
}

impl Parse for TableColumn {
    fn parse(input: ParseStream) -> Result<Self> {
        let table = Ident::parse(&input)?;
        let _: token::Dot = input.parse()?;
        let column = Ident::parse(&input)?;

        Ok(TableColumn {
            table: sqltk::parser::ast::Ident::new(table.to_string()),
            column: sqltk::parser::ast::Ident::new(column.to_string()),
        })
    }
}

impl Parse for NativeSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let _: kw::Native = input.parse()?;
        if input.peek(token::Paren) {
            let content;
            parenthesized!(content in input);
            Ok(NativeSpec(Some(TableColumn::parse(&content)?)))
        } else {
            Ok(NativeSpec(None))
        }
    }
}

impl Parse for ProjectionColumnSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let spec = TypeSpec::parse(input)?;
        if input.peek(token::As) {
            let _: token::As = input.parse()?;
            let alias = Ident::parse(input)?;
            return Ok(ProjectionColumnSpec(
                spec.into(),
                Some(sqltk::parser::ast::Ident::new(alias.to_string())),
            ));
        }
        Ok(ProjectionColumnSpec(spec.into(), None))
    }
}

impl Parse for ProjectionSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        braced!(content in input);
        let specs =
            Punctuated::<ProjectionColumnSpec, Token![,]>::parse_separated_nonempty(&content)?;
        Ok(ProjectionSpec(Vec::from_iter(specs)))
    }
}

impl Parse for ArraySpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        bracketed!(content in input);

        Ok(ArraySpec(Box::new(TypeSpec::parse(&content)?)))
    }
}

impl Parse for FunctionSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let schema = Ident::parse(input)?;
        let _: token::Dot = input.parse()?;
        let function_name = Ident::parse(input)?;

        let generic_args = if input.peek(token::Lt) {
            GenericArgs::parse(input)?.vars
        } else {
            Vec::new()
        };

        let content;
        parenthesized!(content in input);

        let args: Vec<TypeSpec> =
            Punctuated::<TypeSpec, Token![,]>::parse_separated_nonempty(input)?
                .into_iter()
                .collect();

        let _: token::RArrow = input.parse()?;

        let ret = TypeSpec::parse(&content)?;

        let bounds: Vec<_> = if input.peek(token::Where) {
            let _: token::Where = input.parse()?;
            let boundeds = Punctuated::<Bounded, Token![,]>::parse_separated_nonempty(input)?;
            boundeds.into_iter().collect()
        } else {
            vec![]
        };

        Ok(FunctionSpec {
            name: ObjectName(vec![
                sqltk::parser::ast::Ident::new(schema.to_string()),
                sqltk::parser::ast::Ident::new(function_name.to_string()),
            ]),
            inner: GeneralizedFunctionSpec {
                generic_args,
                args,
                ret: ret.into(),
                bounds,
            },
        })
    }
}

struct GenericArgs {
    vars: Vec<TVar>,
}

impl Parse for GenericArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let _: token::Lt = input.parse()?;
        let vars: Vec<TVar> = Punctuated::<TVar, Token![,]>::parse_separated_nonempty(input)?
            .into_iter()
            .collect();
        let _: token::Gt = input.parse()?;
        Ok(Self { vars })
    }
}

impl Parse for BinaryOpSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let generic_args = if input.peek(token::Lt) {
            GenericArgs::parse(input)?.vars
        } else {
            Vec::new()
        };

        let content;
        parenthesized!(content in input);
        let lhs = TypeSpec::parse(&content)?;
        let op = SqltkBinOp::parse(&content)?.0;
        let rhs = TypeSpec::parse(&content)?;

        let _: token::RArrow = input.parse()?;
        let ret = TypeSpec::parse(&input)?;

        let bounds: Vec<_> = if input.peek(token::Where) {
            let _: token::Where = input.parse()?;
            let boundeds = Punctuated::<Bounded, Token![,]>::parse_separated_nonempty(input)?;
            boundeds.into_iter().collect()
        } else {
            vec![]
        };

        Ok(BinaryOpSpec {
            op,
            inner: GeneralizedFunctionSpec {
                generic_args,
                args: vec![lhs, rhs],
                ret: ret.into(),
                bounds,
            },
        })
    }
}

pub(crate) struct SqltkBinOp(pub(crate) BinaryOperator);

impl Parse for SqltkBinOp {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(token::RArrow) {
            let _: token::RArrow = input.parse()?;
            if input.peek(token::Gt) {
                let _: token::Gt = input.parse()?;
                return Ok(Self(::sqltk::parser::ast::BinaryOperator::LongArrow));
            } else {
                return Ok(Self(::sqltk::parser::ast::BinaryOperator::Arrow));
            }
        }

        if input.peek(token::At) {
            let _: token::At = input.parse()?;
            let _: token::Gt = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::AtArrow));
        }

        if input.peek(token::Le) {
            let _: token::Le = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::LtEq));
        }

        if input.peek(token::Lt) {
            let _: token::Lt = input.parse()?;
            if input.peek(token::At) {
                let _: token::At = input.parse()?;
                return Ok(Self(::sqltk::parser::ast::BinaryOperator::ArrowAt));
            } else if input.peek(token::Gt) {
                let _: token::Gt = input.parse()?;
                return Ok(Self(::sqltk::parser::ast::BinaryOperator::NotEq));
            }
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::Lt));
        }

        if input.peek(kw::AND) {
            let _: kw::AND = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::And));
        }

        if input.peek(kw::OR) {
            let _: kw::OR = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::Or));
        }

        if input.peek(token::Ge) {
            let _: token::Ge = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::GtEq));
        }

        if input.peek(token::Eq) {
            let _: token::Eq = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::Eq));
        }

        if input.peek(token::Gt) {
            let _: token::Gt = input.parse()?;
            return Ok(Self(::sqltk::parser::ast::BinaryOperator::Gt));
        }

        Err(syn::Error::new(
            input.span(),
            format!(
                "Expected an operator corresponding to one of the EQL traits Eq, Ord, Bloom or Json"
            ),
        ))
    }
}

impl Parse for EqlTerm {
    fn parse(input: ParseStream) -> Result<Self> {
        let _: kw::EQL = input.parse()?;

        let content;
        parenthesized!(content in input);

        let table = Ident::parse(&content)?;
        let table = sqltk::parser::ast::Ident::new(table.to_string());
        let _: token::Dot = content.parse()?;
        let column = Ident::parse(&content)?;
        let column = sqltk::parser::ast::Ident::new(column.to_string());

        if content.peek(token::Colon) {
            let _: token::Colon = content.parse()?;
            let bounds = EqlTraits::parse(&content)?;

            Ok(EqlTerm::Full(EqlValue(
                TableColumn { table, column },
                bounds,
            )))
        } else {
            Ok(EqlTerm::Full(EqlValue(
                TableColumn { table, column },
                EqlTraits::none(),
            )))
        }
    }
}

// impl Parse for TypeEnv {
//     fn parse(input: ParseStream) -> Result<Self> {
//         let mut env = TypeEnv::new();
//         let specs = Punctuated::<TypeSpec, Token![;]>::parse_terminated(input)?;
//         for spec in specs {
//             env.add(spec);
//         }

//         Ok(env)
//     }
// }
