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
                let ty = self.get_type(select_item);
                match select_item {
                    SelectItem::UnnamedExpr(Expr::Identifier(ident)) => {
                        Ok(ProjectionColumn::new(ty, Some(ident.clone())))
                    }

                    SelectItem::UnnamedExpr(Expr::CompoundIdentifier(object_name)) => Ok(
                        ProjectionColumn::new(ty, Some(object_name.last().cloned().unwrap())),
                    ),

                    SelectItem::UnnamedExpr(expr) => match expr {
                        // For an unnamed expression that is a function call the name of the function becomes the alias.
                        Expr::Function(Function { name, .. }) => Ok(ProjectionColumn::new(
                            ty,
                            Some(name.0.last().unwrap().clone()),
                        )),
                        _ => Ok(ProjectionColumn::new(ty, None)),
                    },

                    SelectItem::ExprWithAlias { alias, .. } => {
                        Ok(ProjectionColumn::new(ty, Some(alias.clone())))
                    }

                    #[allow(unused_variables)]
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

                        Ok(ProjectionColumn::new(ty, None))
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

                        Ok(ProjectionColumn::new(ty, None))
                    }
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.unify_node_with_type(
            select_items,
            Type::Constructor(Constructor::Projection(Projection::new(projection_columns)))
                .into_type_cell(),
        )?;

        Ok(())
    }
}
