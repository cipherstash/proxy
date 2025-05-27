use sqltk::parser::ast::{
    CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName,
};

pub(crate) fn wrap_in_1_arg_function(expr: Expr, name: ObjectName) -> Expr {
    Expr::Function(Function {
        name,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            args: vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))],
            duplicate_treatment: None,
            clauses: vec![],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
        uses_odbc_syntax: false,
    })
}

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
