use crate::{
    inference::{
        type_error::TypeError,
        unifier::{Constructor, Type},
        InferType,
    },
    unifier::{EqlValue, NativeValue, Value},
    ColumnKind, TableColumn, TypeInferencer,
};
use sqlparser::ast::{Ident, Insert};

impl<'ast> InferType<'ast, Insert> for TypeInferencer<'ast> {
    fn infer_enter(&mut self, insert: &'ast Insert) -> Result<(), TypeError> {
        let Insert {
            table_name,
            table_alias,
            columns,
            source,
            ..
        } = insert;

        if table_alias.is_some() {
            return Err(TypeError::UnsupportedSqlFeature("INSERT with ALIAS".into()));
        }

        let table_name: &Ident = table_name.0.last().unwrap();

        let table_columns = columns
            .iter()
            .map(|c| self.schema.resolve_table_column(table_name, c))
            .collect::<Result<Vec<_>, _>>()?;

        let target_columns = Type::projection(
            &table_columns
                .into_iter()
                .map(|stc| {
                    let tc = TableColumn {
                        table: stc.table.clone(),
                        column: stc.column.clone(),
                    };

                    let value_ty = if stc.kind == ColumnKind::Native {
                        Value::Native(NativeValue(Some(tc.clone())))
                    } else {
                        Value::Eql(EqlValue(tc.clone()))
                    };

                    (
                        Type::Constructor(Constructor::Value(value_ty)),
                        Some(tc.column.clone()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        if let Some(source) = source {
            self.unify_node_with_type(&**source, &target_columns)?;
        }

        Ok(())
    }

    fn infer_exit(&mut self, insert: &'ast Insert) -> Result<(), TypeError> {
        let Insert { returning, .. } = insert;

        match returning {
            Some(returning) => {
                self.unify_nodes(insert, returning)?;
            }

            None => {
                self.unify_node_with_type(insert, &Type::empty_projection())?;
            }
        }

        Ok(())
    }
}
