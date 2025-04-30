use std::{collections::HashMap, fmt::Debug};

use sqltk::{
    parser::{
        ast::{self as ast, Statement},
        dialect::PostgreSqlDialect,
        parser::Parser,
    },
    NodeKey,
};
use tracing_subscriber::fmt::format;
use tracing_subscriber::fmt::format::FmtSpan;

use std::sync::Once;

use crate::{Projection, ProjectionColumn, TypeCheckedStatement};

#[allow(unused)]
pub(crate) fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_span_events(FmtSpan::ACTIVE)
            .with_file(true)
            .event_format(format().pretty())
            .pretty()
            .with_test_writer() // ensures it writes to stdout/stderr even during `cargo test`
            .init();
    });
}

pub(crate) fn parse(statement: &str) -> Statement {
    Parser::parse_sql(&PostgreSqlDialect {}, statement).unwrap()[0].clone()
}

pub(crate) fn id(ident: &str) -> ast::Ident {
    ast::Ident::from(ident)
}

pub(crate) fn get_node_key_of_json_selector<'ast>(
    typed: &TypeCheckedStatement<'ast>,
    selector: &'static str,
) -> NodeKey<'ast> {
    typed
        .find_nodekey_for_value_node(ast::Value::SingleQuotedString(selector.into()))
        .expect("could not find selector Value node")
}

pub(crate) fn dummy_encrypted_json_selector<'ast>(
    typed: &TypeCheckedStatement<'ast>,
    selector: &'static str,
) -> HashMap<NodeKey<'ast>, ast::Value> {
    HashMap::from_iter(vec![(
        get_node_key_of_json_selector(typed, selector),
        ast::Value::SingleQuotedString(format!("<encrypted-selector({})>", selector)),
    )])
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
        Projection::Empty => Projection::Empty,
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
