//! Defines macros specifically for reducing the amount of boilerplate in `eql-mapper`.

mod trace_infer;
use quote::{quote, ToTokens};
use trace_infer::*;
mod parse_type_decl;

use proc_macro::TokenStream;

use crate::parse_type_decl::{
    BinaryOpDecls, ConcreteTyArgs, FunctionDecls, ShallowInitTypes, TVar, TypeDecl, TypeEnvDecl
};

/// Generates `#[tracing::instrument]` attributes for `InferType::infer_enter` & `InferType::infer_enter`
/// implementations on `TypeInferencer`.
///
/// This attribute MUST be defined on the trait `impl` itself (not the trait method impls).
#[proc_macro_attribute]
pub fn trace_infer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    trace_infer_(_attr, item)
}

/// Parses a `;`-separated block of binary operator declarations, like this:
///
/// ```ignore
/// let ops: Vec<BinaryOpDecl> = binary_operators! {
///     <T>(T = T) -> Native where T: Eq;
///     <T>(T -> <T as JsonLike>::Accessor) -> T where T: JsonLike;
///     <T>(T <@ T) -> Native where T: Contain;
///     <T>(T ~~ <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch;
///     // ...
/// };
///
#[proc_macro]
pub fn binary_operators(tokens: TokenStream) -> TokenStream {
    let binops = syn::parse_macro_input!(tokens as BinaryOpDecls);
    binops.to_token_stream().into()
}

/// Parses a `;`-separated block of function declarations, like this:
///
/// ```ignore
/// let items: Vec<FunctionDecl> = functions! {
///     pg_catalog.count<T>(T) -> Native;
///     pg_catalog.min<T>(T) -> T where T: Ord;
///     pg_catalog.max<T>(T) -> T where T: Ord;
///     pg_catalog.jsonb_path_query<J>(J, <J as JsonLike>::Path) -> J where J: JsonLike;
/// };
/// ```
#[proc_macro]
pub fn functions(tokens: TokenStream) -> TokenStream {
    let functions = syn::parse_macro_input!(tokens as FunctionDecls);
    functions.to_token_stream().into()
}

/// Builds a [`TypeDecl`] from type declaration syntax. Useful for avoiding boilerplate, especially in tests.
///
/// The generated code is guaranteed not to panic.
///
/// ```ignore
/// let eql_ty: TypeDecl = ty!(EQL(customer.email));
/// let native: TypeDecl = ty!(Native);
/// let projection: TypeDecl = ty!({Native(customer.id) as id, EQL(customer.email: Eq) as email});
/// let array: TypeDecl = ty!([EQL(customer.email: Eq)]);
/// ```
#[proc_macro]
pub fn ty(tokens: TokenStream) -> TokenStream {
    let type_decl = syn::parse_macro_input!(tokens as TypeDecl);
    type_decl.to_token_stream().into()
}

/// Builds a concrete type from type declaration syntax. Useful for avoiding boilerplate, especially in tests.
///
/// WARNING: this macro generates code that will panic if type instantiation fails so limit its usage to setting up
/// tests.
///
/// ```ignore
/// let eql_ty: crate::Type = concrete_ty!(EQL(customer.email));
/// let native: crate::Type = concrete_ty!(Native);
/// let projection: crate::Type = concrete_ty!({Native(customer.id) as id, EQL(customer.email: Eq) as email});
/// let projection: crate::Projection = concrete_ty!({Native(customer.id) as id, EQL(customer.email: Eq) as email} as crate::Projection);
/// let array: crate::Type = concrete_ty!([EQL(customer.email: Eq)]);
/// ```
#[proc_macro]
pub fn concrete_ty(tokens: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(tokens as ConcreteTyArgs);
    let type_decl = &args.ty_decl;
    if let Some(ty_as) = &args.ty_as {
        quote! {{
            let mut unifier = crate::inference::unifier::Unifier::new(
                std::rc::Rc::new(std::cell::RefCell::new(crate::inference::TypeRegistry::new()))
            );
            let ty_as: #ty_as = #type_decl.instantiate_concrete().unwrap().resolved_as(&mut unifier).unwrap();
            ty_as
        }}.into()
    } else {
        quote! {{
            let mut unifier = crate::inference::unifier::Unifier::new(
                std::rc::Rc::new(std::cell::RefCell::new(crate::inference::TypeRegistry::new()))
            );
            use crate::inference::unifier::ResolveType;
            #type_decl.instantiate_concrete().unwrap().resolve_type(&mut unifier).unwrap()
        }}.into()
    }
}

/// Parses a list of pseudo-Rust let bindings where the right hand of the `=` is type declaration syntax (i.e. can be
/// parsed with [`macro@ty`]) and assigns an initialised `Arc<Type>` to each binding.
///
/// WARNING: this macro generates code that will panic if type instantiation fails so it is recommended to limit its
/// usage to setting up tests.
///
/// The type declarations are immediatly converted to `Arc<Type>` values using `InstantiateType::instantiate_shallow`
/// and assigned to a local variable binding in the current scope.
///
/// ```ignore
/// let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
///
/// shallow_init_types! {&mut unifier, {
///     let lhs = T;
///     let rhs = Native;
///     let expected = Native;
/// }};
///
/// let actual = unifier.unify(lhs, rhs).unwrap();
/// assert_eq!(actual, expected);
/// ```
#[proc_macro]
pub fn shallow_init_types(tokens: TokenStream) -> TokenStream {
    let shallow_init_types = syn::parse_macro_input!(tokens as ShallowInitTypes);
    shallow_init_types.to_token_stream().into()
}

/// Shortcut for creating a named type variable. Does not save much boilerplate but is easier on the eye.
///
/// ```ignore
/// // this:
/// let var: TVar = tvar!(A);
///
/// // is sugar for this:
/// let var: TVar = TVar("A".into());
/// ```
#[proc_macro]
pub fn tvar(tokens: TokenStream) -> TokenStream {
    let tvar = syn::parse_macro_input!(tokens as TVar);
    tvar.to_token_stream().into()
}

/// Builds a type environment from a set of `;`-separated type equations. This helps to reduce boilerplate in tests.
///
/// The left hand side of the equation is always a type variable, the right hand side is any type declaration.
///
/// ```ignore
/// let env = type_env! {
///     P = {A as id, B as name, C as email};
///     A = Native(customer.id);
///     B = EQL(customer.name: Eq);
///     C = EQL(customer.email: Eq);
/// };
/// ```
#[proc_macro]
pub fn type_env(tokens: TokenStream) -> TokenStream {
    let env = syn::parse_macro_input!(tokens as TypeEnvDecl);
    env.to_token_stream().into()
}
