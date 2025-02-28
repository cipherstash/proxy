use sqlparser::ast::{Expr, Select, SelectItem, WildcardAdditionalOptions};

use crate::{
    inference::type_error::TypeError, inference::InferType, inference::Type, TypeInferencer,
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
                    projection_columns.push((self.get_type(expr), None))
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

        self.unify(self.get_type(select), Type::projection(&projection_columns))?;

        Ok(())
    }
}
