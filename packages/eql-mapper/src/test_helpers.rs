use std::{collections::HashMap, convert::Infallible, fmt::Debug, ops::ControlFlow};

use sqltk::{
    parser::{
        ast::{self as ast, ObjectNamePart, Statement, Value},
        dialect::PostgreSqlDialect,
        parser::Parser,
    },
    AsNodeKey, Break, NodeKey, Visitable, Visitor,
};
use tracing_subscriber::fmt::format;
use tracing_subscriber::fmt::format::FmtSpan;

use std::sync::Once;

use crate::unifier::{Projection, ProjectionColumn};

#[allow(unused)]
pub(crate) fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_span_events(FmtSpan::ACTIVE)
            // .with_env_filter(EnvFilter::new("eql-mapper::UNIFY=trace,eql-mapper::EVENT_SUBSTITUTE=trace,eql-mapper::INFER_EXIT=trace,eql-mapper::TYPE_ENV=trace"))
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

pub(crate) fn object_name(ident: &str) -> ast::ObjectName {
    ast::ObjectName(vec![ObjectNamePart::Identifier(ast::Ident::from(ident))])
}

pub(crate) fn get_node_key_of_json_selector<'ast>(
    statement: &'ast Statement,
    selector: &Value,
) -> NodeKey<'ast> {
    find_nodekey_for_value_node(statement, selector.clone())
        .expect("could not find selector Value node")
}

pub(crate) fn dummy_encrypted_json_selector(
    statement: &Statement,
    selectors: Vec<Value>,
) -> HashMap<NodeKey<'_>, ast::Value> {
    let mut dummy_encrypted_values = HashMap::new();
    for selector in selectors.into_iter() {
        if let Value::SingleQuotedString(s) = &selector {
            dummy_encrypted_values.insert(
                get_node_key_of_json_selector(statement, &selector),
                ast::Value::SingleQuotedString(format!("<encrypted-selector({s})>")),
            );
        } else {
            panic!("dummy_encrypted_json_selector only works on Value::SingleQuotedString")
        }
    }

    dummy_encrypted_values
}

/// Utility for finding the [`NodeKey`] of a [`Value`] node in `statement` by providing a `matching` equal node to search for.
pub(crate) fn find_nodekey_for_value_node(
    statement: &Statement,
    matching: ast::Value,
) -> Option<NodeKey<'_>> {
    struct FindNode<'ast> {
        needle: ast::Value,
        found: Option<NodeKey<'ast>>,
    }

    impl<'a> Visitor<'a> for FindNode<'a> {
        type Error = Infallible;

        fn enter<N: Visitable>(&mut self, node: &'a N) -> ControlFlow<Break<Self::Error>> {
            if let Some(haystack) = node.downcast_ref::<ast::Value>() {
                if haystack == &self.needle {
                    self.found = Some(haystack.as_node_key());
                    return ControlFlow::Break(Break::Finished);
                }
            }
            ControlFlow::Continue(())
        }
    }

    let mut visitor = FindNode {
        needle: matching,
        found: None,
    };

    let _ = statement.accept(&mut visitor);

    visitor.found
}

#[macro_export]
macro_rules! col {
    ((NATIVE)) => {
        ProjectionColumn {
            ty: Arc::new(Type::Value(Value::Native(NativeValue(None)))),
            alias: None,
        }
    };

    ((NATIVE as $alias:ident)) => {
        ProjectionColumn {
            ty: Arc::new(Type::Value(Value::Native(NativeValue(None)))),
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
            ty: Arc::new(Type::Value(Value::Native(NativeValue(Some(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            }))))),
            alias: Some(id(stringify!($alias))),
        }
    };

    ((EQL($table:ident . $column:ident $(: $($eql_traits:ident)*)?))) => {
        ProjectionColumn {
            ty: Arc::new(Type::Value(Value::Eql(EqlTerm::Full(EqlValue(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            }, $crate::to_eql_traits!($($($eql_traits)*)?)))))),
            alias: None,
        }
    };

    ((EQL($table:ident . $column:ident $(: $($eql_traits:ident)*)?) as $alias:ident)) => {
        ProjectionColumn {
            ty: Arc::new(Type::Value(Value::Eql(EqlTerm::Full(EqlValue(TableColumn {
                table: id(stringify!($table)),
                column: id(stringify!($column)),
            }, $crate::to_eql_traits!($($($eql_traits)*)?)))))),
            alias: Some(id(stringify!($alias))),
        }
    };
}

#[macro_export]
macro_rules! to_eql_traits {
    () => { $crate::unifier::EqlTraits::default() };

    ($($traits:ident)*) => {
        EqlTraits::from_iter(vec![$($crate::unifier::EqlTrait::$traits,)*])
    };
}

#[macro_export]
macro_rules! projection {
    [$($column:tt),*] => { Projection::new(vec![$($crate::col!($column)),*]) };
}

pub fn ignore_aliases(t: &Projection) -> Projection {
    Projection(
        t.0.iter()
            .map(|pc| ProjectionColumn {
                ty: pc.ty.clone(),
                alias: None,
            })
            .collect(),
    )
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
