use crate::{
    get_sql_binop_rule,
    inference::{unifier::Type, InferType, TypeError},
    SqlIdent, TypeInferencer,
};
use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{AccessExpr, Array, Expr, Ident, Subscript};

#[trace_infer]
impl<'ast> InferType<'ast, Expr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, return_val: &'ast Expr) -> Result<(), TypeError> {
        match return_val {
            // Resolve an identifier using the scope, except if it happens to to be the DEFAULT keyword
            // in which case we resolve it to a fresh type variable.
            Expr::Identifier(ident) => {
                // sqltk_parser treats the `DEFAULT` keyword in expression position as an identifier.
                if SqlIdent(ident) == SqlIdent(&Ident::new("default")) {
                    self.unify_node_with_type(return_val, self.fresh_tvar())?;
                } else {
                    self.unify_node_with_type(return_val, self.resolve_ident(ident)?)?;
                };
            }

            Expr::CompoundIdentifier(idents) => {
                self.unify_node_with_type(return_val, self.resolve_compound_ident(idents)?)?;
            }

            Expr::Wildcard(_) => {
                self.unify_node_with_type(return_val, self.resolve_wildcard()?)?;
            }

            Expr::QualifiedWildcard(object_name, _) => {
                self.unify_node_with_type(
                    return_val,
                    self.resolve_qualified_wildcard(object_name)?,
                )?;
            }

            Expr::JsonAccess { .. } => {
                return Err(TypeError::UnsupportedSqlFeature(
                    "Snowflake-style JSON access".into(),
                ))
            }

            Expr::IsFalse(expr)
            | Expr::IsNotFalse(expr)
            | Expr::IsTrue(expr)
            | Expr::IsNotTrue(expr)
            | Expr::IsNull(expr)
            | Expr::IsNotNull(expr)
            | Expr::IsUnknown(expr)
            | Expr::IsNotUnknown(expr) => {
                self.unify_node_with_type(
                    return_val,
                    self.unify(self.get_node_type(&**expr), Type::native())?,
                )?;
            }

            Expr::IsDistinctFrom(a, b) | Expr::IsNotDistinctFrom(a, b) => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_nodes(&**a, &**b)?;
            }

            Expr::InList {
                expr,
                list,
                negated: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(
                    &**expr,
                    list.iter().try_fold(self.get_node_type(&**expr), |a, b| {
                        self.unify(a, self.get_node_type(b))
                    })?,
                )?;
            }

            Expr::InSubquery {
                expr,
                subquery,
                negated: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                let ty = Type::projection(&[(self.get_node_type(&**expr), None)]);
                self.unify_node_with_type(&**subquery, ty)?;
            }

            Expr::InUnnest { .. } => {
                return Err(TypeError::UnsupportedSqlFeature("IN UNNEST".into()))
            }

            Expr::Between {
                expr,
                negated: _,
                low,
                high,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**high, self.unify_nodes(&**expr, &**low)?)?;
            }

            Expr::BinaryOp { left, op, right } => {
                get_sql_binop_rule(op).apply_constraints(self, left, right, return_val)?;
            }

            //customer_name LIKE 'A%';
            Expr::Like {
                negated: _,
                expr,
                pattern,
                escape_char: _,
                any: false,
            }
            | Expr::ILike {
                negated: _,
                expr,
                pattern,
                escape_char: _,
                any: false,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_nodes(&**expr, &**pattern)?;
            }

            Expr::Like { any: true, .. } | Expr::ILike { any: true, .. } => {
                Err(TypeError::UnsupportedSqlFeature(
                    "Snowflake-specific feature: ANY in LIKE/ILIKE".into(),
                ))?
            }

            Expr::SimilarTo {
                negated: _,
                expr,
                pattern,
                escape_char: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_nodes_with_type(&**expr, &**pattern, Type::native())?;
            }

            Expr::RLike { .. } => Err(TypeError::UnsupportedSqlFeature(
                "MySQL-specific feature: RLIKE".into(),
            ))?,

            Expr::AnyOp {
                left,
                compare_op: _,
                right,
                is_some: _,
            }
            | Expr::AllOp {
                left,
                compare_op: _,
                right,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_nodes(&**left, &**right)?;
            }

            Expr::Ceil { expr, .. }
            | Expr::Floor { expr, .. }
            | Expr::UnaryOp { expr, .. }
            | Expr::Convert { expr, .. }
            | Expr::Cast { expr, .. } => {
                self.unify_nodes_with_type(return_val, &**expr, Type::native())?;
            }

            Expr::AtTimeZone {
                timestamp,
                time_zone,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**timestamp, Type::native())?;
                self.unify_node_with_type(&**time_zone, Type::native())?;
            }

            Expr::Extract {
                field: _,
                syntax: _,
                expr,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
            }

            Expr::Position { expr, r#in } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_nodes_with_type(&**expr, &**r#in, Type::native())?;
            }

            Expr::Substring {
                expr,
                substring_from,
                substring_for,
                special: _,
                shorthand: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
                if let Some(expr) = substring_from {
                    self.unify_node_with_type(&**expr, Type::native())?;
                }
                if let Some(expr) = substring_for {
                    self.unify_node_with_type(&**expr, Type::native())?;
                }
            }

            // Similar to Overlay but apply constrainst to all in vec
            // SELECT TRIM(BOTH '*' FROM '***Hello, World!***') AS star_trimmed;
            Expr::Trim {
                expr,
                trim_where,
                trim_what,
                trim_characters,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
                if let Some(trim_where) = trim_where {
                    self.unify_node_with_type(trim_where, Type::native())?;
                }
                if let Some(trim_what) = trim_what {
                    self.unify_node_with_type(&**trim_what, Type::native())?;
                }
                if let Some(trim_characters) = trim_characters {
                    self.unify_all_with_type(trim_characters, Type::native())?;
                }
            }

            Expr::Overlay {
                expr,
                overlay_what,
                overlay_from,
                overlay_for,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
                self.unify_node_with_type(&**overlay_what, Type::native())?;
                self.unify_node_with_type(&**overlay_from, Type::native())?;
                if let Some(overlay_for) = overlay_for {
                    self.unify_node_with_type(&**overlay_for, Type::native())?;
                }
            }

            Expr::Collate { expr, collation: _ } => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
            }

            // The current `Expr` shares the same type hole as the sub-expression
            Expr::Nested(expr) => {
                self.unify_nodes(return_val, &**expr)?;
            }

            Expr::Value(value) => {
                self.unify_nodes(return_val, value)?;
            }

            Expr::TypedString {
                data_type: _,
                value: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
            }

            // The return type of this function and the return type of this expression must be the same type.
            Expr::Function(function) => {
                self.unify_node_with_type(return_val, self.get_node_type(function))?;
            }

            // When operand is Some(operand), all conditions must be of the same type as the operand and much support equality
            // When operand is None, all conditions must be native (they are boolean)
            // The elements of `results` and else_result must be the same type
            // The type of the overall expression is the type of the results/else_result
            Expr::Case {
                operand,
                conditions,
                else_result,
            } => {
                let result_ty = self.fresh_tvar();

                match operand {
                    Some(operand) => {
                        for cond_when in conditions {
                            self.unify_nodes_with_type(
                                return_val,
                                &**operand,
                                self.unify_node_with_type(&cond_when.condition, self.fresh_tvar())?,
                            )?;
                        }
                    }
                    None => {
                        for cond_when in conditions {
                            self.unify_node_with_type(&cond_when.condition, Type::native())?;
                        }
                    }
                }

                for cond_when in conditions {
                    self.unify_node_with_type(&cond_when.result, result_ty.clone())?;
                }

                if let Some(else_result) = else_result {
                    self.unify_node_with_type(else_result, result_ty.clone())?;
                };

                self.unify_node_with_type(return_val, result_ty)?;
            }

            Expr::Exists {
                subquery: _,
                negated: _,
            } => {
                self.unify_node_with_type(return_val, Type::native())?;
            }

            Expr::Subquery(subquery) => {
                self.unify_nodes(return_val, &**subquery)?;
            }

            // unsupported SQL features
            Expr::GroupingSets(_) | Expr::Cube(_) | Expr::Rollup(_) => {
                Err(TypeError::UnsupportedSqlFeature(
                    "Unsupported SQL feature: grouping sets/cube/rollup".into(),
                ))?
            }

            // The type system does not yet support tuple types.
            Expr::Tuple(_) => Err(TypeError::UnsupportedSqlFeature(
                "Tuple types are not yet supported".into(),
            ))?,

            Expr::Struct {
                values: _,
                fields: _,
            } => Err(TypeError::UnsupportedSqlFeature(
                "BigQuery-specific struct syntax".into(),
            ))?,

            Expr::Named { expr: _, name: _ } => Err(TypeError::UnsupportedSqlFeature(
                "BigQuery-specific named expression".into(),
            ))?,

            Expr::Dictionary(_) | Expr::Map(_) => Err(TypeError::UnsupportedSqlFeature(
                "DuckDB-specific map/dictionary syntax".into(),
            ))?,

            // This expression type represents a chain of field and/or array subscripting.  EQL Mapper does not support
            // compound object field access yet so this will fail with a TypeError::Unsupported for object field access.
            // The type of a CompoundFieldAccess expression is the type of the element returned by the last array access
            // in the chain.
            Expr::CompoundFieldAccess { root, access_chain } => {
                let mut root_ty = self.fresh_tvar();
                let mut access_ty = self.fresh_tvar();

                for access_expr in access_chain.iter() {
                    match access_expr {
                        AccessExpr::Subscript(Subscript::Index { index }) => {
                            access_ty = self.fresh_tvar();
                            root_ty = Type::array(access_ty.clone());
                            self.unify_node_with_type(index, Type::native())?;
                        }
                        AccessExpr::Subscript(Subscript::Slice {
                            lower_bound,
                            upper_bound,
                            stride,
                        }) => {
                            self.unify_node_with_type(lower_bound, Type::native())?;
                            self.unify_node_with_type(upper_bound, Type::native())?;
                            self.unify_node_with_type(stride, Type::native())?;
                            access_ty = self.fresh_tvar();
                            root_ty = Type::array(access_ty.clone());
                        }
                        AccessExpr::Dot(_) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "field access of compound value".into(),
                            ))
                        }
                    }
                }

                self.unify_node_with_type(return_val, access_ty)?;
                self.unify_node_with_type(&**root, root_ty)?;
            }

            Expr::Array(Array { elem, named: _ }) => {
                // Constrain all elements of the array to be the same type.
                let elem_ty = self.unify_all_with_type(elem, self.fresh_tvar())?;
                let array_ty = Type::array(elem_ty);
                self.unify_node_with_type(return_val, array_ty)?;
            }

            // interval is unmapped, value is unmapped
            Expr::Interval(interval) => {
                self.unify_node_with_type(return_val, Type::native())?;
                self.unify_node_with_type(&*interval.value, Type::native())?;
            }

            // mysql specific
            Expr::MatchAgainst {
                columns: _,
                match_value: _,
                opt_search_modifier: _,
            } => Err(TypeError::UnsupportedSqlFeature(
                "MySQL-specific match against".into(),
            ))?,

            Expr::OuterJoin(_) => Err(TypeError::UnsupportedSqlFeature(
                "Unsupported SQL feature: old outer join syntax using `(+)`".into(),
            ))?,

            Expr::Prior(_) => Err(TypeError::UnsupportedSqlFeature(
                "Unsupported SQL feature: CONNECT BY".into(),
            ))?,

            Expr::Lambda(_) => Err(TypeError::UnsupportedSqlFeature(
                "Unsupported SQL feature: lambda functions".into(),
            ))?,

            Expr::IsNormalized {
                expr: _,
                form: _,
                negated: _,
            } => Err(TypeError::UnsupportedSqlFeature(
                "Unsupported SQL feature: <expr> IS [ NOT ] [ form ] NORMALIZED".into(),
            ))?,

            Expr::Prefixed {
                prefix: _,
                value: _,
            } => Err(TypeError::UnsupportedSqlFeature(
                "Unsupported SQL feature: prefixed expressions".into(),
            ))?,
        }

        Ok(())
    }
}
