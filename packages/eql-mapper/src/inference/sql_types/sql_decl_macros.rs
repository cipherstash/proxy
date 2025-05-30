// #[macro_export]
// macro_rules! sql_fn_args {
//     () => { vec![] };

//     ($arg:ident $(,$rest:ident)*) => {
//         vec![$crate::to_type_spec!($arg) $(, $crate::to_type_spec!($rest))*]
//     };
// }

// #[macro_export]
// macro_rules! sql_fn {
//     (
//         $schema:ident . $name:ident ( $( $arg:ident $(, $($arg_rest:ident)+ )* )? ) -> $return_kind:ident
//     ) => {
//         $crate::inference::sql_types::ExplicitSqlFunctionRule::new(
//             $crate::inference::sql_types::CompoundIdent::from(&vec![
//                 ::sqltk::parser::ast::Ident::new(stringify!($schema)),
//                 ::sqltk::parser::ast::Ident::new(stringify!($name))
//             ]),
//             $crate::inference::sql_types::FunctionSig::new(
//                 $crate::sql_fn_args!($( $arg $(, $($arg_rest)+ )* )?),
//                 $crate::to_type_spec!($return_kind),
//                 $crate::to_type_env!( $( $arg $(, $($arg_rest)+ )* ,)? $return_kind ),
//             ),
//         )
//     };

//     (
//         $schema:ident . $name:ident ( $( $arg:ident $(, $($arg_rest:ident)+ )* )? ) -> $return_kind:ident where $($bounds:tt)+
//     ) => {
//         (
//             $crate::inference::sql_types::ExplicitSqlFunctionRule::new(
//                 $crate::inference::sql_types::CompoundIdent::from(&vec![
//                     ::sqltk::parser::ast::Ident::new(stringify!($schema)),
//                     ::sqltk::parser::ast::Ident::new(stringify!($name))
//                 ]),
//                 $crate::inference::sql_types::FunctionSig::new(
//                     $crate::sql_fn_args!($( $arg $(, $($arg_rest)+ )* )?),
//                     $crate::to_type_spec!($return_kind),
//                     $crate::to_type_env!( $( $arg $(, $($arg_rest)+ )* ,)? $return_kind, $($bounds)+ ),
//                 ),
//             )
//         )
//     };
// }

// #[macro_export]
// macro_rules! sql_binop_literal {
//     ((->)) => {
//         ::sqltk::parser::ast::BinaryOperator::Arrow
//     };
//     ((->>)) => {
//         ::sqltk::parser::ast::BinaryOperator::LongArrow
//     };
//     ((@>)) => {
//         ::sqltk::parser::ast::BinaryOperator::AtArrow
//     };
//     ((<@)) => {
//         ::sqltk::parser::ast::BinaryOperator::ArrowAt
//     };
//     ((@?)) => {
//         ::sqltk::parser::ast::BinaryOperator::AtQuestion
//     };
//     (AND) => {
//         ::sqltk::parser::ast::BinaryOperator::And
//     };
//     (OR) => {
//         ::sqltk::parser::ast::BinaryOperator::Or
//     };
//     (=) => {
//         ::sqltk::parser::ast::BinaryOperator::Eq
//     };
//     (>) => {
//         ::sqltk::parser::ast::BinaryOperator::Gt
//     };
//     (>=) => {
//         ::sqltk::parser::ast::BinaryOperator::GtEq
//     };
//     (<) => {
//         ::sqltk::parser::ast::BinaryOperator::Lt
//     };
//     (<=) => {
//         ::sqltk::parser::ast::BinaryOperator::LtEq
//     };
//     ((<>)) => {
//         ::sqltk::parser::ast::BinaryOperator::NotEq
//     };
// }

// #[macro_export]
// macro_rules! arg_syntax {
//     () => {
//         None
//     };

//     ($arg:tt) => {
//         Some($arg)
//     };
// }

// #[macro_export]
// macro_rules! to_type_spec {
//     (Native) => {
//         $crate::unifier::TypeSpec::Native
//     };

//     ($($arg:tt)+) => {
//         $crate::unifier::TypeSpec::Var(stringify!($($arg)+))
//     };
// }

// #[macro_export]
// macro_rules! to_type_env {
//     // $crate::to_type_env!( $lhs_ty$(::$lhs_assoc)?, $rhs_ty$(::$rhs_assoc)?, $ret$(::$rhs_assoc)?, $( $ty: $trait $(+ $traits)* )?),

//     () => { $crate::inference::TypeEnv::new() };

//     ($env:ident @add $ty:ident:$assoc_ty:ident $($rest:tt)*) => {
//         {
//             $env.add_type($crate::to_type_spec!($ty::$assoc_ty), None);
//             $crate::to_type_env!($env @add $($rest:tt)*);
//             env
//         }
//     };

//     ($($tokens:tt)+) => {
//         {
//             let mut env = $crate::inference::TypeEnv::new();
//             $crate::to_type_env!($env @add $($tokens:tt)+)
//         }
//     };

//     ($env:ident @add) => { $env }
// }

// #[macro_export]
// macro_rules! binop {
//     (
//         ($lhs:ident $(::$lhs_assoc:ident)? $op:tt $rhs:ident $(::$rhs_assoc:ident)?) -> $ret:ident$(::$ret_assoc:ident)?
//         $(
//             where $generic:ident: $trait:ident $(+ $traits:ident )*
//         )?
//     ) => {
//         (
//             $crate::sql_binop_literal!($op),
//             $crate::inference::sql_types::ExplicitBinaryOpRule::new(
//                 $crate::sql_binop_literal!($op),
//                 $crate::to_type_env!( $lhs$(::$lhs_assoc)?, $rhs$(::$rhs_assoc)?, $ret$(::$rhs_assoc)?, $( $generic: $trait $(+ $traits)* )?),
//                 $crate::to_type_spec!($lhs),
//                 $crate::to_type_spec!($rhs),
//                 $crate::to_type_spec!($ret),
//             ),
//         )
//     };
// }
