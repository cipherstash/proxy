use std::fmt::Debug;

use sqlparser::{
    ast::{self as ast, Statement},
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::{Projection, ProjectionColumn};

pub(crate) fn parse(statement: &'static str) -> Statement {
    Parser::parse_sql(&PostgreSqlDialect {}, statement).unwrap()[0].clone()
}

pub(crate) fn id(ident: &str) -> ast::Ident {
    ast::Ident::from(ident)
}

#[macro_export]
macro_rules! col {
    ((NATIVE)) => {
        ProjectionColumn {
            ty: Value::Native(NativeValue(None)),
            alias: None,
        }
    };

    ((NATIVE as $alias:ident)) => {
        ProjectionColumn {
            ty: Value::Native(NativeValue(None)),
            alias: Some(id(stringify!($alias))),
        }
    };

    ((NATIVE($table:ident . $column:ident))) => {
        ProjectionColumn {
            ty: Value::Native(NativeValue(Some(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            }))),
            alias: None,
        }
    };

    ((NATIVE($table:ident . $column:ident) as $alias:ident)) => {
        ProjectionColumn {
            ty: Value::Native(NativeValue(Some(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            }))),
            alias: Some(id(stringify!($alias))),
        }
    };

    ((EQL($table:ident . $column:ident))) => {
        ProjectionColumn {
            ty: Value::Eql(EqlValue::from((stringify!($table), stringify!($column)))),
            alias: None,
        }
    };

    ((EQL($table:ident . $column:ident) as $alias:ident)) => {
        ProjectionColumn {
            ty: Value::Eql(EqlValue(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            })),
            alias: Some(id(stringify!($alias))),
        }
    };
}

#[macro_export]
macro_rules! projection {
    [$($column:tt),*] => { Projection::new(vec![$(col!($column)),*]) };
}

pub fn ignore_aliases(t: &Projection) -> Projection {
    match t {
        Projection::WithColumns(columns) => Projection::WithColumns(
            columns
                .iter()
                .map(|pc| ProjectionColumn {
                    ty: pc.ty.clone(),
                    alias: None,
                })
                .collect(),
        ),
        Projection::Empty => t.clone(),
    }
}

pub fn assert_transitive_eq<T: Eq + Debug>(items: &[T]) {
    for window in items.windows(2) {
        match &window {
            &[a, b] => {
                assert_eq!(a, b);
            }
            _ => {
                panic!("assert_transitive_eq requires a minimum of two items");
            }
        }
    }
}
