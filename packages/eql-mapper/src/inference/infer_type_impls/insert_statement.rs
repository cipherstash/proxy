use std::sync::Arc;

use crate::{
    inference::{
        type_error::TypeError,
        unifier::{Constructor, Type},
        InferType,
    },
    unifier::{EqlValue, NativeValue, Value},
    ColumnKind, TableColumn, TypeInferencer,
};
use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Ident, Insert};

#[trace_infer]
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

        let table_columns = if columns.is_empty() {
            // When no columns are specified, the source must unify with a projection of ALL table columns.
            self.table_resolver.resolve_table_columns(table_name)?
        } else {
            columns
                .iter()
                .map(|c| self.table_resolver.resolve_table_column(table_name, c))
                .collect::<Result<Vec<_>, _>>()?
        };

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
                        Arc::new(Type::Constructor(Constructor::Value(value_ty))),
                        Some(tc.column.clone()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        if let Some(source) = source {
            self.unify_node_with_type(&**source, target_columns)?;
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
                self.unify_node_with_type(insert, Type::empty_projection())?;
            }
        }

        Ok(())
    }
}
