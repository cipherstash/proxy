#[macro_export]
macro_rules! to_kind {
    (NATIVE) => {
        $crate::Kind::Native
    };
    ($generic:ident) => {
        $crate::Kind::Generic(stringify!($generic))
    };
}

#[macro_export]
macro_rules! sql_fn_args {
    (()) => { vec![] };

    (($arg:ident)) => { vec![$crate::to_kind!($arg)] };

    (($arg:ident $(,$rest:ident)*)) => {
        vec![$crate::to_kind!($arg) $(, $crate::to_kind!($rest))*]
    };
}

#[macro_export]
macro_rules! sql_fn {
    ($name:ident $args:tt -> $return_kind:ident) => {
        $crate::SqlFunction::new(
            stringify!($name),
            FunctionSig::new($crate::sql_fn_args!($args), $crate::to_kind!($return_kind)),
        )
    };
}
