use std::{cell::RefCell, rc::Rc};

use sqlparser::ast::{Ident, Insert};

use crate::{
    inference::type_error::TypeError,
    inference::InferType,
    inference::{Constructor, Def, Status, Type},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Insert> for TypeInferencer {
    fn infer_enter(&mut self, insert: &'ast Insert) -> Result<(), TypeError> {
        let Insert {
            table_name,
            table_alias,
            columns,
            source,
            returning,
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

        if let Some(source) = source {
            let target_columns = Type::projection(
                &table_columns
                    .into_iter()
                    .map(|tc| {
                        (
                            Rc::new(RefCell::new(Type(
                                Def::Constructor(Constructor::Scalar(tc.column.ty.clone())),
                                Status::Resolved,
                            ))),
                            Some(tc.column.name.clone()),
                        )
                    })
                    .collect::<Vec<_>>(),
            );

            self.unify(target_columns, self.get_type(&**source))?;
        }

        match returning {
            Some(returning) => {
                self.unify(self.get_type(insert), self.get_type(returning))?;
            }

            None => {
                self.unify(self.get_type(insert), Type::empty())?;
            }
        }

        Ok(())
    }
}
