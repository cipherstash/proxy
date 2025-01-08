use sqlparser::{
    ast::{self as ast, Statement},
    dialect::PostgreSqlDialect,
    parser::Parser,
};

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
