use std::{cell::RefCell, rc::Rc};

use sqlparser::ast::{Expr, Function, Select, SelectItem, WildcardAdditionalOptions};

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    unifier::{Constructor, Def, ProjectionColumn, Status},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        let mut projection_columns: Vec<(_, _)> = Vec::new();

        for select_item in select.projection.iter() {
            match select_item {
                SelectItem::UnnamedExpr(Expr::Identifier(ident)) => projection_columns.push((
                    self.scope.borrow().resolve_ident(ident)?,
                    Some(ident.clone()),
                )),

                SelectItem::UnnamedExpr(Expr::CompoundIdentifier(object_name)) => {
                    projection_columns.push((
                        self.scope.borrow().resolve_compound_ident(object_name)?,
                        Some(object_name.last().cloned().unwrap()),
                    ))
                }

                SelectItem::UnnamedExpr(expr) => {
                    match expr {
                        Expr::Function(Function { name, .. }) => {
                            projection_columns.push((self.get_type(expr), Some(name.0.last().unwrap().clone())))
                        }
                        _ => {
                            projection_columns.push((self.get_type(expr), None))
                        }
                    }
                }

                SelectItem::ExprWithAlias { expr, alias } => {
                    projection_columns.push((self.get_type(expr), Some(alias.clone())))
                }

                SelectItem::QualifiedWildcard(object_name, options) => {
                    let WildcardAdditionalOptions {
                        opt_ilike: None,
                        opt_exclude: None,
                        opt_except: None,
                        opt_replace: None,
                        opt_rename: None,
                    } = options
                    else {
                        return Err(TypeError::UnsupportedSqlFeature(
                            "options on wildcard".into(),
                        ));
                    };

                    projection_columns.push((
                        self.scope
                            .borrow()
                            .resolve_qualified_wildcard(&object_name.0)?,
                        None,
                    ));
                }

                SelectItem::Wildcard(options) => {
                    let WildcardAdditionalOptions {
                        opt_ilike: None,
                        opt_exclude: None,
                        opt_except: None,
                        opt_replace: None,
                        opt_rename: None,
                    } = options
                    else {
                        return Err(TypeError::UnsupportedSqlFeature(
                            "options on wildcard".into(),
                        ));
                    };

                    projection_columns.push((self.scope.borrow().resolve_wildcard()?, None));
                }
            }
        }

        let status = if projection_columns
            .iter()
            .all(|col| col.0.borrow().status() == Status::Resolved)
        {
            Status::Resolved
        } else {
            Status::Partial
        };

        self.unify_and_log(
            select,
            self.get_type(select),
            Rc::new(RefCell::new(Type(
                Def::Constructor(Constructor::Projection(ProjectionColumn::vec_of(
                    &projection_columns,
                ))),
                status,
            ))),
        )?;

        Ok(())
    }
}
