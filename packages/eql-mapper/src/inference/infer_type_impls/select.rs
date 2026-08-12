use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{
    Distinct, Expr, GroupByExpr, JoinConstraint, JoinOperator, Select, SelectItem,
};

use super::query_statement::resolve_positional_key;
use crate::unifier::{Projection, Type, Value};
use crate::{
    inference::{type_error::TypeError, InferType},
    EqlTrait, TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_nodes(select, &select.projection)?;

        // `SELECT ... INTO` copies the projection into a brand-new table the
        // schema knows nothing about. An encrypted column landing there would
        // be raw ciphertext in a table the mapper can never resolve — no
        // decryption on read, no term rewrites on query — so reject it rather
        // than let the data silently escape the schema. A projection of only
        // native columns is passed through untouched.
        if select.into.is_some() {
            let ty = self.get_node_type(&select.projection);
            let ty = ty.follow_tvars(&self.unifier.borrow());
            if let Type::Value(Value::Projection(projection)) = &*ty {
                if Self::projection_contains_eql(projection) {
                    return Err(TypeError::UnsupportedSqlFeature(
                        "SELECT INTO with an encrypted column".into(),
                    ));
                }
            }
        }

        // `WHERE`, `HAVING` and join `ON` conditions are boolean expressions,
        // and booleans are always native — every EQL comparison produces a
        // native result. Pin the condition to `Native` so that a bare literal
        // or placeholder condition (`WHERE true`, `ON true`, `WHERE $1`) is
        // typed where the clause is inferred instead of relying on the late
        // unresolved-value fallback, and so that an encrypted value can never
        // itself be the condition.
        if let Some(selection) = &select.selection {
            self.unify_node_with_type(selection, Type::native())?;
        }

        if let Some(having) = &select.having {
            self.unify_node_with_type(having, Type::native())?;
        }

        for table_with_joins in &select.from {
            for join in &table_with_joins.joins {
                let constraint = match &join.join_operator {
                    JoinOperator::Join(constraint)
                    | JoinOperator::Inner(constraint)
                    | JoinOperator::Left(constraint)
                    | JoinOperator::LeftOuter(constraint)
                    | JoinOperator::Right(constraint)
                    | JoinOperator::RightOuter(constraint)
                    | JoinOperator::FullOuter(constraint)
                    | JoinOperator::Semi(constraint)
                    | JoinOperator::LeftSemi(constraint)
                    | JoinOperator::RightSemi(constraint)
                    | JoinOperator::Anti(constraint)
                    | JoinOperator::LeftAnti(constraint)
                    | JoinOperator::RightAnti(constraint)
                    | JoinOperator::StraightJoin(constraint) => Some(constraint),

                    JoinOperator::AsOf {
                        match_condition,
                        constraint,
                    } => {
                        self.unify_node_with_type(match_condition, Type::native())?;
                        Some(constraint.as_ref())
                    }

                    JoinOperator::CrossJoin
                    | JoinOperator::CrossApply
                    | JoinOperator::OuterApply => None,
                };

                if let Some(JoinConstraint::On(condition)) = constraint {
                    self.unify_node_with_type(condition, Type::native())?;
                }
            }
        }

        // Deduplication is equality, so every expression `DISTINCT` dedupes on
        // must support it. For an encrypted column that means its domain has to
        // carry an equality term — `eql_v3_boolean`, for instance, is
        // storage-only and cannot be deduplicated at all.
        //
        // Without this bound the requirement goes unchecked and the failure is
        // silent rather than loud: PostgreSQL would dedupe on the raw jsonb
        // payload, whose ciphertext is randomised per row, so every row looks
        // distinct and `DISTINCT` degrades into a no-op.
        match &select.distinct {
            Some(Distinct::Distinct) => {
                for item in &select.projection {
                    match item {
                        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                            self.unify_node_with_eq_bound(expr)?;
                        }

                        // A wildcard has no per-column expression to bind, but
                        // it still projects the columns `DISTINCT` dedupes on —
                        // and they are the ones most easily overlooked, being
                        // invisible in the query text. Check the columns the
                        // wildcard resolved to instead.
                        _ => self.check_projected_columns_support_eq(item)?,
                    }
                }
            }

            Some(Distinct::On(exprs)) => {
                for expr in exprs {
                    self.unify_node_with_eq_bound(expr)?;
                }
            }

            None => {}
        }

        // Grouping is equality, so every `GROUP BY` key needs an equality term.
        // As with `ORDER BY`, a key written as an ordinal is resolved against
        // the projection — otherwise `GROUP BY 1` over an encrypted column is
        // unconstrained and every row becomes its own group.
        if let GroupByExpr::Expressions(exprs, _) = &select.group_by {
            for expr in exprs {
                let key = resolve_positional_key(Some(select), expr);
                self.unify_node_with_bound(key, EqlTrait::Eq)?;

                // A key written as a literal (`GROUP BY 1`) reaches the
                // database as a plain constant — PostgreSQL only accepts
                // integer ordinals here — so the literal itself is always
                // native, independently of the projected column it selects.
                if matches!(expr, Expr::Value(_)) {
                    self.unify_node_with_type(expr, Type::native())?;
                }
            }
        }

        Ok(())
    }
}

impl<'ast> TypeInferencer<'ast> {
    /// Whether any column of `projection` — including the columns of a nested
    /// projection, as produced by a wildcard — is an encrypted column.
    ///
    /// Callers must have followed type variables first (via
    /// [`Type::follow_tvars`]), which resolves column types recursively.
    fn projection_contains_eql(projection: &Projection) -> bool {
        projection.columns().iter().any(|column| match &*column.ty {
            Type::Value(Value::Projection(nested)) => Self::projection_contains_eql(nested),
            Type::Value(Value::Eql(_)) => true,
            _ => false,
        })
    }

    /// Constrains `node` to a type that implements [`EqlTrait::Eq`].
    ///
    /// A native type satisfies this trivially; the bound only bites for an
    /// encrypted column, whose domain must carry an equality term.
    fn unify_node_with_eq_bound(&mut self, node: &'ast Expr) -> Result<(), TypeError> {
        let bounded = self
            .unifier
            .borrow_mut()
            .fresh_bounded_tvar(EqlTrait::Eq.into());

        // Unify the node's *resolved* type with the bound, rather than just
        // pointing the node at a bounded variable: a variable satisfies any
        // bound vacuously, so binding alone would defer the check indefinitely
        // and never reject the column.
        let unified = self.unify(self.get_node_type(node), bounded)?;
        self.unify_node_with_type(node, unified)?;

        Ok(())
    }

    /// Requires every encrypted column a wildcard projects to implement
    /// [`EqlTrait::Eq`].
    ///
    /// The wildcard resolves to a projection type rather than to expressions, so
    /// there is no node to bind a variable to — the traits are read off the
    /// resolved columns directly.
    fn check_projected_columns_support_eq(
        &mut self,
        item: &'ast SelectItem,
    ) -> Result<(), TypeError> {
        let ty = self.get_node_type(item);

        let Type::Value(Value::Projection(projection)) = &*ty else {
            return Ok(());
        };

        Self::check_projection_supports_eq(projection)
    }

    /// Walks a projection, requiring every encrypted column in it to implement
    /// [`EqlTrait::Eq`].
    ///
    /// A wildcard's projection nests — one entry per relation in the `FROM`,
    /// each holding that relation's columns — so this recurses rather than
    /// looking only at the top level.
    fn check_projection_supports_eq(projection: &Projection) -> Result<(), TypeError> {
        for column in projection.columns() {
            match &*column.ty {
                Type::Value(Value::Projection(nested)) => {
                    Self::check_projection_supports_eq(nested)?
                }

                Type::Value(Value::Eql(eql_term)) if !eql_term.eql_value().trait_impls().eq => {
                    return Err(TypeError::UnsatisfiedBounds(
                        column.ty.clone(),
                        EqlTrait::Eq.into(),
                    ))
                }

                _ => {}
            }
        }

        Ok(())
    }
}
