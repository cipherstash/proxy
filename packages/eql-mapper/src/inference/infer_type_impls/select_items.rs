use sqlparser::ast::{Expr, Function, SelectItem, WildcardAdditionalOptions};

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    unifier::{Constructor, Projection, ProjectionColumn},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Vec<SelectItem>> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_items: &'ast Vec<SelectItem>) -> Result<(), TypeError> {
        let projection_columns: Vec<ProjectionColumn> = select_items
            .iter()
            .map(|select_item| {
                match select_item {
                    SelectItem::UnnamedExpr(Expr::Identifier(ident)) => Ok(ProjectionColumn {
                        ty: self.scope_tracker.borrow().resolve_ident(ident)?,
                        alias: Some(ident.clone()),
                    }),

                    SelectItem::UnnamedExpr(Expr::CompoundIdentifier(object_name)) => {
                        Ok(ProjectionColumn {
                            ty: self
                                .scope_tracker
                                .borrow()
                                .resolve_compound_ident(object_name)?,
                            alias: Some(object_name.last().cloned().unwrap()),
                        })
                    }

                    SelectItem::UnnamedExpr(expr) => match expr {
                        // For an unnamed expression that is a function call the name of the function becomes the alias.
                        Expr::Function(Function { name, .. }) => Ok(ProjectionColumn {
                            ty: self.get_type(expr),
                            alias: Some(name.0.last().unwrap().clone()),
                        }),
                        _ => Ok(ProjectionColumn {
                            ty: self.get_type(expr),
                            alias: None,
                        }),
                    },

                    SelectItem::ExprWithAlias { expr, alias } => Ok(ProjectionColumn {
                        ty: self.get_type(expr),
                        alias: Some(alias.clone()),
                    }),

                    #[allow(unused_variables)]
                    SelectItem::QualifiedWildcard(object_name, options) => {
                        let WildcardAdditionalOptions {
                            opt_ilike: None,
                            opt_exclude: None,
                            opt_except: None,
                            opt_replace: None,
                            opt_rename: None,
                            wildcard_token,
                        } = options
                        else {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "options on wildcard".into(),
                            ));
                        };

                        Ok(ProjectionColumn {
                            ty: self
                                .scope_tracker
                                .borrow()
                                .resolve_qualified_wildcard(&object_name.0)?,
                            alias: None,
                        })
                    }

                    SelectItem::Wildcard(options) => {
                        let WildcardAdditionalOptions {
                            opt_ilike: None,
                            opt_exclude: None,
                            opt_except: None,
                            opt_replace: None,
                            opt_rename: None,
                            wildcard_token: _,
                        } = options
                        else {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "options on wildcard".into(),
                            ));
                        };

                        Ok(ProjectionColumn {
                            ty: self.scope_tracker.borrow().resolve_wildcard()?,
                            alias: None,
                        })
                    }
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.unify_node_with_type(
            select_items,
            &Type::Constructor(Constructor::Projection(Projection::new(projection_columns))),
        )?;

        Ok(())
    }
}
