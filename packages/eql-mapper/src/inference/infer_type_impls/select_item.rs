use std::{cell::RefCell, rc::Rc};

use sqlparser::ast::{Expr, Function, SelectItem, WildcardAdditionalOptions};

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    unifier::{Constructor, Def, ProjectionColumn},
    TypeInferencer,
};

impl<'ast> InferType<'ast, SelectItem> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_item: &'ast SelectItem) -> Result<(), TypeError> {
        let projection_column = match select_item {
            SelectItem::UnnamedExpr(Expr::Identifier(ident)) => ProjectionColumn {
                ty: self.scope.borrow().resolve_ident(ident)?,
                alias: Some(ident.clone()),
            },

            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(object_name)) => ProjectionColumn {
                ty: self.scope.borrow().resolve_compound_ident(object_name)?,
                alias: Some(object_name.last().cloned().unwrap()),
            },

            SelectItem::UnnamedExpr(expr) => match expr {
                // For an unnamed expression that is a function call the name of the function becomes the alias.
                Expr::Function(Function { name, .. }) => ProjectionColumn {
                    ty: self.get_type(expr),
                    alias: Some(name.0.last().unwrap().clone()),
                },
                _ => ProjectionColumn {
                    ty: self.get_type(expr),
                    alias: None,
                },
            },

            SelectItem::ExprWithAlias { expr, alias } => ProjectionColumn {
                ty: self.get_type(expr),
                alias: Some(alias.clone()),
            },

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

                ProjectionColumn {
                    ty: self
                        .scope
                        .borrow()
                        .resolve_qualified_wildcard(&object_name.0)?,
                    alias: None,
                }
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

                ProjectionColumn {
                    ty: self.scope.borrow().resolve_wildcard()?,
                    alias: None,
                }
            }
        };

        let status = projection_column.ty.borrow().status();

        // There is no Constructor variant for ProjectionColumn so we return a single column projection to hold the
        // projection column.  If embedded in a multicolumn projection later then the projection will be flattened (just
        // like with wildcards).  Additionally, a single column projection is unifiable with a Scalar or an Array.
        self.unify_and_log(
            select_item,
            self.get_type(select_item),
            Rc::new(RefCell::new(Type(
                Def::Constructor(Constructor::Projection(Rc::new(RefCell::new(vec![
                    projection_column,
                ])))),
                status,
            ))),
        )?;

        Ok(())
    }
}
