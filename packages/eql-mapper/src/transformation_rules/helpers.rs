use sqltk::parser::ast::{CastKind, DataType, Expr, Ident, ObjectName};

pub(crate) fn cast_as_encrypted(wrapped: sqltk::parser::ast::Value) -> Expr {
    let cast_jsonb = Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(Expr::Value(wrapped)),
        data_type: DataType::JSONB,
        format: None,
    };

    let encrypted_type = ObjectName(vec![Ident::new("eql_v2_encrypted")]);

    Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(cast_jsonb),
        data_type: DataType::Custom(encrypted_type, vec![]),
        format: None,
    }
}
