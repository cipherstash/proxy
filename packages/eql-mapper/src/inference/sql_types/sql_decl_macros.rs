#[macro_export]
macro_rules! sql_fn_args {
    () => { vec![] };

    ($arg:ident $(,$rest:ident)*) => {
        vec![$crate::to_type_arg!($arg) $(, $crate::to_type_arg!($rest))*]
    };
}

#[macro_export]
macro_rules! sql_fn {
    (
        $schema:ident . $name:ident ( $( $arg:ident $(, $($arg_rest:ident)+ )* )? ) -> $return_kind:ident
    ) => {
        $crate::inference::sql_types::ExplicitSqlFunctionRule::new(
            $crate::inference::sql_types::CompoundIdent::from(&vec![
                ::sqltk::parser::ast::Ident::new(stringify!($schema)),
                ::sqltk::parser::ast::Ident::new(stringify!($name))
            ]),
            $crate::inference::sql_types::FunctionSig::new(
                $crate::sql_fn_args!($( $arg $(, $($arg_rest)+ )* )?),
                $crate::to_type_arg!($return_kind),
                $crate::to_type_env!( $( $arg $(, $($arg_rest)+ )* ,)? $return_kind ),
            ),
        )
    };

    (
        $schema:ident . $name:ident ( $( $arg:ident $(, $($arg_rest:ident)+ )* )? ) -> $return_kind:ident where $($bounds:tt)+
    ) => {
        (
            $crate::inference::sql_types::ExplicitSqlFunctionRule::new(
                $crate::inference::sql_types::CompoundIdent::from(&vec![
                    ::sqltk::parser::ast::Ident::new(stringify!($schema)),
                    ::sqltk::parser::ast::Ident::new(stringify!($name))
                ]),
                $crate::inference::sql_types::FunctionSig::new(
                    $crate::sql_fn_args!($( $arg $(, $($arg_rest)+ )* )?),
                    $crate::to_type_arg!($return_kind),
                    $crate::to_type_env!( $( $arg $(, $($arg_rest)+ )* ,)? $return_kind, $($bounds)+ ),
                ),
            )
        )
    };
}

#[macro_export]
macro_rules! sql_binop_literal {
    ((->)) => {
        ::sqltk::parser::ast::BinaryOperator::Arrow
    };
    ((->>)) => {
        ::sqltk::parser::ast::BinaryOperator::LongArrow
    };
    ((@>)) => {
        ::sqltk::parser::ast::BinaryOperator::AtArrow
    };
    ((<@)) => {
        ::sqltk::parser::ast::BinaryOperator::ArrowAt
    };
    ((@?)) => {
        ::sqltk::parser::ast::BinaryOperator::AtQuestion
    };
    (AND) => {
        ::sqltk::parser::ast::BinaryOperator::And
    };
    (OR) => {
        ::sqltk::parser::ast::BinaryOperator::Or
    };
    (=) => {
        ::sqltk::parser::ast::BinaryOperator::Eq
    };
    (>) => {
        ::sqltk::parser::ast::BinaryOperator::Gt
    };
    (>=) => {
        ::sqltk::parser::ast::BinaryOperator::GtEq
    };
    (<) => {
        ::sqltk::parser::ast::BinaryOperator::Lt
    };
    (<=) => {
        ::sqltk::parser::ast::BinaryOperator::LtEq
    };
    ((<>)) => {
        ::sqltk::parser::ast::BinaryOperator::NotEq
    };
}

#[macro_export]
macro_rules! arg_syntax {
    () => {
        None
    };

    ($arg:tt) => {
        Some($arg)
    };
}

#[macro_export]
macro_rules! to_type_arg {
    (Native) => {
        TypeArg::Native
    };

    ($($arg:tt)+) => {
        TypeArg::Generic(stringify!($($arg)+))
    };
}

#[macro_export]
macro_rules! to_type_env {
    (@add $env:ident $generic:ident: $bound:ident $(,$($rest:tt)+)?) => {
        $env.add_type_arg($crate::to_type_arg!($generic), Some($crate::to_trait_bound!($bound)));
        $( $crate::to_type_env!(@add $env $($rest)+) )?
    };

    (@add $env:ident $generic:ident: $bound:ident<$type_arg:ident> $(,$($rest:tt)+)?) => {
        $env.add_type_arg($crate::to_type_arg!($generic), Some($crate::to_trait_bound!($bound<$type_arg>)));
        $( $crate::to_type_env!(@add $env $($rest)+) )?
    };

    (@add $env:ident $generic:ident $(, $($rest:tt)*)?) => {
        $env.add_type_arg($crate::to_type_arg!($generic), None);
        $( $crate::to_type_env!(@add $env $($rest)+) )?
    };

    ($generic:ident $(, $($rest:tt)*)?) => {
        {
            let mut env = $crate::inference::TypeEnv::new();
            $crate::to_type_env!(@add env $generic $(, $($rest)*)?);
            env
        }
    };

    ($generic:ident: $($rest:tt)+) => {
        {
            let mut env = $crate::inference::TypeEnv::new();
            $crate::to_type_env!(@add env $generic: $($rest)+);
            env
        }
    };

    () => {
        $crate::inference::TypeEnv::new()
    }
}

#[macro_export]
macro_rules! to_trait_bound {
    ($trait_:ident) => { TraitBound::WithoutParam(EqlTrait::$trait_) };
    ($trait_:ident<$type_arg:ident>) => { TraitBound::WithOneParam($crate::to_type_arg!($type_arg), EqlTrait::$trait_) };
}

#[macro_export]
macro_rules! binop {
    (($lhs:ident $op:tt $rhs:ident) -> $ret:ident )=> {
        (
            $crate::sql_binop_literal!($op),
            $crate::inference::sql_types::ExplicitBinaryOpRule::new(
                $crate::sql_binop_literal!($op),
                $crate::to_type_env!( $lhs, $rhs, $ret ),
                $crate::to_type_arg!($lhs),
                $crate::to_type_arg!($rhs),
                $crate::to_type_arg!($ret),
            ),
        )
    };

    (($lhs:ident $op:tt $rhs:ident) -> $ret:ident where $generic:ident: $($rest:tt)+ ) => {
        (
            $crate::sql_binop_literal!($op),
            $crate::inference::sql_types::ExplicitBinaryOpRule::new(
                $crate::sql_binop_literal!($op),
                $crate::to_type_env!( $lhs, $rhs, $ret, $generic: $($rest)+ ),
                $crate::to_type_arg!($lhs),
                $crate::to_type_arg!($rhs),
                $crate::to_type_arg!($ret),
            ),
        )
    };
}