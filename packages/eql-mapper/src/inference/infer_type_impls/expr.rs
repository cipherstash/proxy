use crate::{
    get_sql_binop_rule,
    inference::{
        unifier::{EqlTerm, EqlValue, TokenType, Type, Value},
        InferType, TypeError,
    },
    json_value_selector::{json_accessor, json_accessor_chain, unnest},
    EqlTrait, IdentCase, JsonSelectorSegment, JsonSelectorSource, Param, TypeInferencer,
};
use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{self as ast, AccessExpr, Array, BinaryOperator, Expr, Ident, Subscript};

/// The capability a comparison operator requires of its operands, or `None` if
/// it is not a comparison.
fn comparison_capability(op: &BinaryOperator) -> Option<EqlTrait> {
    match op {
        BinaryOperator::Eq | BinaryOperator::NotEq => Some(EqlTrait::Eq),
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq => {
            Some(EqlTrait::Ord)
        }
        _ => None,
    }
}

#[trace_infer]
impl<'ast> InferType<'ast, Expr> for TypeInferencer<'ast> {
    /// Marks JSON accessor chains that a comparison will fuse, on the way DOWN.
    ///
    /// Typing is post-order, so by the time a chain's outermost `->` is typed its
    /// parent is not yet known — and the parent is exactly what decides WHERE the
    /// chain's composed path has to be recorded. Under `= $1` the chain is fused
    /// into the equality's own needle and the accessor is discarded, so the path
    /// belongs to the value operand; anywhere else the chain collapses to a
    /// surviving accessor whose selector must carry the path itself. Recording the
    /// intent here is what lets the `->` rule pick the right channel.
    ///
    /// Both outcomes are legal — this no longer gates whether a chain is allowed,
    /// only which record it produces. Writing the path into both channels would be
    /// worse than writing it into neither: the fused case would then also try to
    /// resolve the discarded selector as a standalone path, which for
    /// `j -> $1 -> 'b' = $2` is unresolvable at Parse time and would refuse a
    /// query that works today.
    ///
    /// Syntactic only: no child has a type yet. Whether the chain's root is
    /// really an encrypted document is checked on the way back up.
    fn infer_enter(&mut self, expr_val: &'ast Expr) -> Result<(), TypeError> {
        if let Expr::BinaryOp { left, op, right } = expr_val {
            // Only EQUALITY fuses a chain, collapsing the whole path into one
            // value-selector containment against the root document so that the
            // accessor disappears from the emitted SQL entirely.
            //
            // Ordering does NOT: `RewriteEqlComparisonOps` types the scalar
            // operand as a SteVec ordering term and leaves the accessor standing,
            // so the chain is collapsed by `CollapseJsonAccessorChain` like any
            // other and keeps its own path record.
            let fuses = matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq);

            if fuses {
                for operand in [&**left, &**right] {
                    // Mark every accessor node along the chain's spine, not just
                    // the outermost. `j -> 'a' -> 'b' -> 'c'` is three nested
                    // `BinaryOp`s and EVERY one of them is typed, so marking only
                    // the top would leave `j -> 'a' -> 'b'` looking unfused and
                    // record a path for a selector the fusion then discards.
                    //
                    // Unnest at each step: `((j -> 'a') -> 'b') = $1` reaches the
                    // `->` rule as the bare accessor, so marking the bracket
                    // would mark a node that rule never asks about.
                    // One step at a time: `json_accessor_chain` would jump
                    // straight to the root and skip the intermediates that need
                    // marking.
                    let mut node = unnest(operand);

                    while let Some((container, _)) = json_accessor(node) {
                        self.mark_fusable_json_chain(node);
                        node = unnest(container);
                    }
                }
            }
        }

        Ok(())
    }

    fn infer_exit(&mut self, expr_val: &'ast Expr) -> Result<(), TypeError> {
        match expr_val {
            // Resolve an identifier using the scope, except if it happens to to be the DEFAULT keyword
            // in which case we resolve it to a fresh type variable.
            Expr::Identifier(ident) => {
                // sqltk_parser treats the `DEFAULT` keyword in expression position as an identifier.
                if IdentCase(ident) == IdentCase(&Ident::new("default")) {
                    self.unify_node_with_type(expr_val, self.fresh_tvar())?;
                } else {
                    self.unify_node_with_type(expr_val, self.resolve_ident(ident)?)?;
                };
            }

            Expr::CompoundIdentifier(idents) => {
                self.unify_node_with_type(expr_val, self.resolve_compound_ident(idents)?)?;
            }

            Expr::Wildcard(_) => {
                self.unify_node_with_type(expr_val, self.resolve_wildcard()?)?;
            }

            Expr::QualifiedWildcard(object_name, _) => {
                self.unify_node_with_type(expr_val, self.resolve_qualified_wildcard(object_name)?)?;
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
                    expr_val,
                    self.unify(self.get_node_type(&**expr), Type::native())?,
                )?;
            }

            Expr::IsDistinctFrom(a, b) | Expr::IsNotDistinctFrom(a, b) => {
                let ty = self
                    .unifier
                    .borrow_mut()
                    .fresh_bounded_tvar(EqlTrait::Eq.into());
                self.unify_node_with_type(&**a, ty.clone())?;
                self.unify_node_with_type(&**b, ty.clone())?;
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_nodes(&**a, &**b)?;
            }

            Expr::InList {
                expr,
                list,
                negated: _,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_node_with_type(
                    &**expr,
                    list.iter().try_fold(self.get_node_type(&**expr), |a, b| {
                        self.unify(a, self.get_node_type(b))
                    })?,
                )?;

                // `IN` is equality against each element, so the operand's
                // domain has to carry an equality term. Without this the shape
                // type-checks on a storage-only column and the refusal comes
                // from EQL at the database instead — correct of EQL, but
                // inconsistent with `=` on the same column, which is caught
                // here.
                self.unify_node_with_bound(&**expr, EqlTrait::Eq)?;
            }

            Expr::InSubquery {
                expr,
                subquery,
                negated: _,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                let ty = Type::projection(&[(self.get_node_type(&**expr), None)]);
                self.unify_node_with_type(&**subquery, ty)?;

                // Equality against each returned row, as for `IN (…)`.
                self.unify_node_with_bound(&**expr, EqlTrait::Eq)?;
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
                self.unify_node_with_type(expr_val, Type::native())?;
                let ty = self
                    .unifier
                    .borrow_mut()
                    .fresh_bounded_tvar(EqlTrait::Ord.into());
                self.unify_node_with_type(&**expr, ty.clone())?;
                self.unify_node_with_type(&**low, ty.clone())?;
                self.unify_node_with_type(&**high, ty.clone())?;
            }

            Expr::BinaryOp { left, op, right } => {
                // Encrypted JSON field ORDERING (`col -> sel < value`, `>`, `<=`,
                // `>=`): the value operand is a scalar SteVec ordering term
                // (`{v,i,op}`, `QueryOp::SteVecTerm`), not a JSON document, and the
                // comparison runs through `eql_v3.ord_term` on both sides. Type the
                // operand as `EqlTerm::JsonOrd` so it encrypts and casts as an
                // ordering operand — the generic `T Ord T` rule would instead unify
                // it to the whole JSON type (→ a full document, which cannot be an
                // ordering operand). Equality (`=`) is intentionally NOT handled here
                // (exact JSON equality is value-selector containment, not ordering).
                let handled = if matches!(
                    op,
                    BinaryOperator::Lt
                        | BinaryOperator::LtEq
                        | BinaryOperator::Gt
                        | BinaryOperator::GtEq
                ) {
                    match (self.eql_json_value(left), self.eql_json_value(right)) {
                        (Some(json), None) => {
                            self.unify_node_with_type(
                                &**right,
                                Type::Value(Value::Eql(EqlTerm::JsonOrd(json))),
                            )?;
                            self.unify_node_with_type(expr_val, Type::native())?;
                            true
                        }
                        (None, Some(json)) => {
                            self.unify_node_with_type(
                                &**left,
                                Type::Value(Value::Eql(EqlTerm::JsonOrd(json))),
                            )?;
                            self.unify_node_with_type(expr_val, Type::native())?;
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };

                // Encrypted JSON field EQUALITY (`col -> sel = value`, `<>`).
                // Exact equality is not a term comparison but *value-selector
                // containment*: one keyed MAC over path and value together. Type
                // the value operand `EqlTerm::JsonValueSelector` and record where
                // its path half comes from, so the proxy can fuse the two into a
                // single needle at encryption time (see `JsonValueSelectors`).
                //
                // Unlike the ordering case above this requires a genuine field
                // ACCESS on the JSON side — a bare `col = $1` on a whole
                // encrypted JSON column is document equality and must keep its
                // ordinary typing.
                let handled = handled
                    || if matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq) {
                        match (
                            self.eql_json_field_access(left),
                            self.eql_json_field_access(right),
                        ) {
                            (Some((json, selectors)), None) => {
                                let fused =
                                    self.infer_json_value_selector(json, selectors, right)?;
                                if fused {
                                    self.unify_node_with_type(expr_val, Type::native())?;
                                }
                                fused
                            }
                            (None, Some((json, selectors))) => {
                                let fused =
                                    self.infer_json_value_selector(json, selectors, left)?;
                                if fused {
                                    self.unify_node_with_type(expr_val, Type::native())?;
                                }
                                fused
                            }
                            _ => false,
                        }
                    } else {
                        false
                    };

                // Encrypted JSON field ACCESS (`->`, `->>`).
                //
                // This is the chain-aware half of `EqlTerm::JsonExtracted`. The
                // operator declaration is compositional — it can only see the
                // type of its immediate left operand — but a chain is not
                // compositional: `j -> 'a' -> 'b'` is ONE path into ONE
                // document, and its intermediate `j -> 'a'` has no independent
                // existence for the database. Typing it step by step would make
                // the first link `JsonExtracted` and the second link fail, which
                // would reject every chain.
                //
                // So the rule consults the chain BELOW this node rather than the
                // type of its operand. Within one expression the walker can
                // always reach the root, so being handed an extracted
                // intermediate is fine — what matters is whether the root is a
                // document. Across a subquery boundary the walker cannot reach
                // it, the root is itself `JsonExtracted`, and the access is
                // refused.
                let handled = handled
                    || if matches!(op, BinaryOperator::Arrow | BinaryOperator::LongArrow) {
                        // A root that is already an extracted entry is the
                        // cross-subquery case, at any chain length: `a -> 'foo'`
                        // where `a` is `j -> 'bar'`. Report it precisely instead
                        // of leaving the declaration to say `JsonExtracted` does
                        // not satisfy `JsonLike`.
                        if let Some((root, _)) = json_accessor_chain(expr_val) {
                            if self.is_eql_json_extracted(root) {
                                return Err(TypeError::UnqueryableJsonExtraction);
                            }
                        }

                        // Only a MULTI-step chain needs special treatment. A
                        // single access is handled correctly by the declaration
                        // (`-> <T as JsonLike>::Output`), for native and
                        // encrypted alike.
                        match json_accessor_chain(expr_val)
                            .filter(|(_, selectors)| selectors.len() > 1)
                        {
                            Some((root, selectors)) => match self.eql_json_document(root) {
                                // A chain rooted at a document: type the whole
                                // access as one extraction from that document,
                                // and the OUTERMOST selector as its accessor so
                                // it is encrypted. `CollapseJsonAccessorChain`
                                // then drops the inner accessors, leaving that
                                // one selector to carry the whole path — so
                                // record what the whole path is.
                                //
                                // Unless a comparison above will fuse the chain
                                // into its own needle, in which case the accessor
                                // does not survive at all and the path belongs in
                                // the other channel, recorded by the equality
                                // branch above.
                                Some(json) => {
                                    if !self.is_fusable_json_chain(expr_val) {
                                        self.record_json_accessor_path(&selectors)?;
                                    }

                                    self.unify_node_with_type(
                                        &**right,
                                        Type::Value(Value::Eql(EqlTerm::JsonAccessor(
                                            json.clone(),
                                        ))),
                                    )?;
                                    self.unify_node_with_type(
                                        expr_val,
                                        Type::Value(Value::Eql(EqlTerm::JsonExtracted(json))),
                                    )?;
                                    true
                                }
                                // Native JSON: the declaration is correct for it,
                                // and plaintext `jsonb` chains legitimately.
                                None => false,
                            },
                            None => false,
                        }
                    } else {
                        false
                    };

                if !handled {
                    // `@@` is symmetric in PostgreSQL, so the encrypted column
                    // may be written on either side. The operator rule is
                    // positional (`T @@ <T as TokenMatch>::Tokenized`), so hand
                    // it the operands in the order it expects rather than the
                    // order they were written. Applying them positionally types
                    // the column as the *pattern*: the pattern is then never
                    // encrypted, and the rewrite emits `match_term(pattern) @>
                    // match_term(col)` — a backwards containment that silently
                    // matches nothing.
                    let (lhs, rhs) = if matches!(op, BinaryOperator::AtAt)
                        && self.is_eql_typed(right)
                        && !self.is_eql_typed(left)
                    {
                        (&**right, &**left)
                    } else {
                        (&**left, &**right)
                    };

                    get_sql_binop_rule(op).apply_constraints(self, lhs, rhs, expr_val)?;
                }

                // The operands of a predicate reach PostgreSQL as query
                // operands — terms only, never a ciphertext. Record them so the
                // proxy projects their payloads accordingly. Containment
                // (`@>`/`<@`) is deliberately excluded: its needle is a whole
                // document and keeps its full payload.
                if matches!(
                    op,
                    BinaryOperator::Eq
                        | BinaryOperator::NotEq
                        | BinaryOperator::Lt
                        | BinaryOperator::LtEq
                        | BinaryOperator::Gt
                        | BinaryOperator::GtEq
                        | BinaryOperator::AtAt
                ) {
                    self.record_query_operands([&**left, &**right]);
                }
            }

            // `customer_name LIKE 'A%'`. Route LIKE/ILIKE through the `~~`/`~~*`
            // operator rules so an encrypted LHS must implement `TokenMatch` (the
            // pattern becomes its `Tokenized` type, the result is `Native`).
            // Previously this only unified the result with `Native`, so LIKE on an
            // encrypted column bypassed capability checking entirely.
            Expr::Like {
                negated,
                expr,
                pattern,
                escape_char: _,
                any: false,
            } => {
                let op = if *negated {
                    BinaryOperator::PGNotLikeMatch
                } else {
                    BinaryOperator::PGLikeMatch
                };
                get_sql_binop_rule(&op).apply_constraints(self, expr, pattern, expr_val)?;
                self.record_query_operands([&**expr, &**pattern]);
            }
            Expr::ILike {
                negated,
                expr,
                pattern,
                escape_char: _,
                any: false,
            } => {
                let op = if *negated {
                    BinaryOperator::PGNotILikeMatch
                } else {
                    BinaryOperator::PGILikeMatch
                };
                get_sql_binop_rule(&op).apply_constraints(self, expr, pattern, expr_val)?;
                self.record_query_operands([&**expr, &**pattern]);
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
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_nodes_with_type(&**expr, &**pattern, Type::native())?;
            }

            Expr::RLike { .. } => Err(TypeError::UnsupportedSqlFeature(
                "MySQL-specific feature: RLIKE".into(),
            ))?,

            Expr::AnyOp {
                left,
                compare_op,
                right,
                is_some: _,
            }
            | Expr::AllOp {
                left,
                compare_op,
                right,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_nodes(&**left, &**right)?;

                // `x <op> ANY/ALL (…)` applies `<op>` to every element, so the
                // capability is the operator's. Discarding `compare_op` left
                // both `= ANY` and `> ANY` unconstrained.
                if let Some(eql_trait) = comparison_capability(compare_op) {
                    self.unify_node_with_bound(&**left, eql_trait)?;
                }
            }

            Expr::Ceil { expr, .. }
            | Expr::Floor { expr, .. }
            | Expr::UnaryOp { expr, .. }
            | Expr::Convert { expr, .. }
            | Expr::Cast { expr, .. } => {
                self.unify_nodes_with_type(expr_val, &**expr, Type::native())?;
            }

            Expr::AtTimeZone {
                timestamp,
                time_zone,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_node_with_type(&**timestamp, Type::native())?;
                self.unify_node_with_type(&**time_zone, Type::native())?;
            }

            Expr::Extract {
                field: _,
                syntax: _,
                expr,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
            }

            Expr::Position { expr, r#in } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_nodes_with_type(&**expr, &**r#in, Type::native())?;
            }

            Expr::Substring {
                expr,
                substring_from,
                substring_for,
                special: _,
                shorthand: _,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
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
                self.unify_node_with_type(expr_val, Type::native())?;
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
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
                self.unify_node_with_type(&**overlay_what, Type::native())?;
                self.unify_node_with_type(&**overlay_from, Type::native())?;
                if let Some(overlay_for) = overlay_for {
                    self.unify_node_with_type(&**overlay_for, Type::native())?;
                }
            }

            Expr::Collate { expr, collation: _ } => {
                self.unify_node_with_type(expr_val, Type::native())?;
                self.unify_node_with_type(&**expr, Type::native())?;
            }

            // The current `Expr` shares the same type hole as the sub-expression
            Expr::Nested(expr) => {
                self.unify_nodes(expr_val, &**expr)?;
            }

            Expr::Value(value) => {
                self.unify_nodes(expr_val, value)?;
            }

            Expr::TypedString {
                data_type: _,
                value: _,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
            }

            // The return type of this function and the return type of this expression must be the same type.
            Expr::Function(function) => {
                self.unify_node_with_type(expr_val, self.get_node_type(function))?;
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
                    // `CASE x WHEN y THEN z` compares `x` to each `y` for
                    // equality and returns `z`. The operand and the conditions
                    // share a type; the CASE's own type is `z`'s and must stay
                    // independent of it.
                    //
                    // Unifying `expr_val` with the operand here instead forced
                    // the result to the operand's type, so
                    // `CASE enc WHEN 'a' THEN 1 ELSE 0 END` typed the integer
                    // results as values of the encrypted column and encrypted
                    // them.
                    Some(operand) => {
                        let operand_ty = self.get_node_type(&**operand);

                        for cond_when in conditions {
                            self.unify_node_with_type(&cond_when.condition, operand_ty.clone())?;
                        }

                        // The comparison is equality, so the operand's domain
                        // has to carry an equality term.
                        self.unify_node_with_bound(&**operand, EqlTrait::Eq)?;
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

                self.unify_node_with_type(expr_val, result_ty)?;
            }

            Expr::Exists {
                subquery: _,
                negated: _,
            } => {
                self.unify_node_with_type(expr_val, Type::native())?;
            }

            Expr::Subquery(subquery) => {
                self.unify_nodes(expr_val, &**subquery)?;
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

                self.unify_node_with_type(expr_val, access_ty)?;
                self.unify_node_with_type(&**root, root_ty)?;
            }

            Expr::Array(Array { elem, named: _ }) => {
                // Constrain all elements of the array to be the same type.
                let elem_ty = self.unify_all_with_type(elem, self.fresh_tvar())?;
                let array_ty = Type::array(elem_ty);
                self.unify_node_with_type(expr_val, array_ty)?;
            }

            // interval is unmapped, value is unmapped
            Expr::Interval(interval) => {
                self.unify_node_with_type(expr_val, Type::native())?;
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

impl<'ast> TypeInferencer<'ast> {
    /// If `expr` resolves to an encrypted JSON (`JsonLike`) value — the field
    /// access side of a JSON ordering comparison (`col -> sel`, `col ->> sel`, or
    /// `jsonb_path_query_first(col, sel)`) — return its [`EqlValue`]. Returns
    /// `None` for scalar EQL columns (which compare via the ordinary term rewrite)
    /// and for non-EQL types.
    /// Whether `expr` has resolved to an encrypted value.
    fn is_eql_typed(&self, expr: &'ast Expr) -> bool {
        matches!(&*self.get_node_type(expr), Type::Value(Value::Eql(_)))
    }

    fn eql_json_value(&self, expr: &'ast Expr) -> Option<EqlValue> {
        match &*self.get_node_type(expr) {
            Type::Value(Value::Eql(eql_term)) => {
                let eql_value = eql_term.eql_value();
                (eql_value.domain_identity().token == TokenType::Json).then(|| eql_value.clone())
            }
            _ => None,
        }
    }

    /// An encrypted JSON **document** — something with an `sv` array that a path
    /// can be traversed into.
    ///
    /// Unlike [`Self::eql_json_value`] this inspects the term *variant*, because
    /// the distinction it draws is the whole point of
    /// [`EqlTerm::JsonExtracted`]: an already-extracted entry carries the same
    /// `EqlValue` as the document it came from, so ignoring the variant would
    /// accept it and re-derive the bug. An entry has no `sv`, so traversing it
    /// selects nothing.
    fn eql_json_document(&self, expr: &'ast Expr) -> Option<EqlValue> {
        match &*self.get_node_type(expr) {
            Type::Value(Value::Eql(eql_term @ (EqlTerm::Full(_) | EqlTerm::Partial(_, _)))) => {
                let eql_value = eql_term.eql_value();
                (eql_value.domain_identity().token == TokenType::Json).then(|| eql_value.clone())
            }
            _ => None,
        }
    }

    /// Whether `expr` is an already-extracted encrypted JSON entry.
    ///
    /// Used to turn a second traversal into a precise error rather than letting
    /// it fall through to the operator declaration, where the failure would be
    /// an opaque unsatisfied-`JsonLike` bound.
    fn is_eql_json_extracted(&self, expr: &'ast Expr) -> bool {
        matches!(
            &*self.get_node_type(expr),
            Type::Value(Value::Eql(EqlTerm::JsonExtracted(_)))
        )
    }

    /// Deconstructs an encrypted-JSON **field access** into the accessed value
    /// and the expressions supplying its selectors, outermost last:
    ///
    /// - `col -> sel`, `col ->> sel`
    /// - `jsonb_path_query_first(col, sel)`
    /// - and chains of those: `col -> 'a' -> 'b'`
    ///
    /// Returns `None` for anything else — importantly for a bare encrypted JSON
    /// column, which is a whole document, not a field of one. Equality needs the
    /// selector expressions themselves (not just the type), because the path
    /// they compose is one half of the fused value-selector needle.
    ///
    /// A chain yields ALL of its selectors, not just the outermost. Its
    /// intermediate accessors have no independent existence for the database:
    /// the payload is encrypted, so native `->` applied to it selects nothing.
    /// The chain is one path into one document, and it is fused as one.
    fn eql_json_field_access(&self, expr: &'ast Expr) -> Option<(EqlValue, Vec<&'ast Expr>)> {
        let (root, selectors) = json_accessor_chain(expr)?;

        // Resolved from the ROOT, which must be a whole document. Reading the
        // node's own type instead would accept a chain rooted at an
        // already-extracted entry (`a -> 'foo'` where `a` came from a subquery)
        // and fuse a needle keyed on `$.foo` when the real path is `$.bar.foo`.
        self.eql_json_document(root).map(|json| (json, selectors))
    }

    /// Records each of `exprs` that is a literal or placeholder as a query
    /// operand — an operand of a predicate, which reaches PostgreSQL carrying
    /// only search terms and never a ciphertext.
    ///
    /// Column references are ignored: they are already stored payloads, and it
    /// is only the bound values whose encryption shape this decides.
    fn record_query_operands(&self, exprs: impl IntoIterator<Item = &'ast Expr>) {
        for expr in exprs {
            match Self::as_ast_value(expr) {
                Some(ast::Value::Placeholder(placeholder)) => {
                    if let Ok(param) = Param::try_from(placeholder) {
                        self.record_query_operand_param(param);
                    }
                }
                Some(node) => self.record_query_operand_literal(node),
                None => {}
            }
        }
    }

    /// Types `value` — the value half of `col -> sel = value` — as a fused
    /// value selector, and records where its path half (`selectors`) comes from.
    ///
    /// A path step that is neither a literal nor a placeholder (a column
    /// reference, a function call) cannot be resolved to a needle at encryption
    /// time, so the fusion is declined and the comparison falls through to
    /// ordinary typing — where it will fail the capability check with a clearer
    /// error than a half-built needle would produce. One unresolvable step
    /// declines the whole chain: a partial path is not a path.
    ///
    /// Returns whether the fusion was applied. The caller must not treat a
    /// declined fusion as handled: doing so skips the binop rule that is the
    /// promised fall-through, leaving `value` with an unconstrained type
    /// variable and surfacing an opaque "incomplete type" error instead of the
    /// capability error.
    fn infer_json_value_selector(
        &self,
        json: EqlValue,
        selectors: Vec<&'ast Expr>,
        value: &'ast Expr,
    ) -> Result<bool, TypeError> {
        let Some(segments) = selectors
            .into_iter()
            .map(Self::json_selector_segment)
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(false);
        };

        let source = JsonSelectorSource::new(segments);

        self.unify_node_with_type(
            value,
            Type::Value(Value::Eql(EqlTerm::JsonValueSelector(json))),
        )?;

        match Self::as_ast_value(value) {
            Some(ast::Value::Placeholder(placeholder)) => {
                if let Ok(param) = Param::try_from(placeholder) {
                    self.record_json_value_selector_param(param, source)?;
                }
            }
            Some(node) => self.record_json_value_selector_literal(node, source),
            None => {}
        }

        Ok(true)
    }

    /// Records the composed path of a multi-step accessor chain against the
    /// operand that will carry it: the OUTERMOST selector, the one node of the
    /// chain that survives the rewrite.
    ///
    /// The surviving operand's own text is a single segment (`'b'` of
    /// `j -> 'a' -> 'b'`), while the selector it must key is the whole path
    /// (`$.a.b`). Nothing the proxy is handed at encryption time could recover
    /// the difference, which is why it is recorded here.
    ///
    /// Unlike the fused-equality case, an unresolvable step cannot be waved
    /// through to a capability error: the rewrite collapses the chain either way,
    /// so a step the proxy cannot resolve would be silently dropped and the query
    /// would read a different field. It is refused.
    fn record_json_accessor_path(&self, selectors: &[&'ast Expr]) -> Result<(), TypeError> {
        let segments = selectors
            .iter()
            .map(|selector| Self::json_selector_segment(selector))
            .collect::<Option<Vec<_>>>()
            .ok_or(TypeError::UncomposableJsonPath)?;

        let source = JsonSelectorSource::new(segments);

        // The last selector is the outermost, and `json_selector_segment`
        // succeeded for it above, so it is a literal or a placeholder.
        let Some(outermost) = selectors.last().and_then(|s| Self::as_ast_value(s)) else {
            return Err(TypeError::UncomposableJsonPath);
        };

        match outermost {
            ast::Value::Placeholder(placeholder) => {
                let param =
                    Param::try_from(placeholder).map_err(|_| TypeError::UncomposableJsonPath)?;
                self.record_json_accessor_path_param(param, source)
            }
            node => {
                self.record_json_accessor_path_literal(node, source);
                Ok(())
            }
        }
    }

    /// Classifies one step of the path half of a fused value selector: a
    /// placeholder yields the param it will arrive in, a literal yields its text
    /// inline.
    fn json_selector_segment(selector: &'ast Expr) -> Option<JsonSelectorSegment> {
        match Self::as_ast_value(selector)? {
            ast::Value::Placeholder(placeholder) => Param::try_from(placeholder)
                .ok()
                .map(JsonSelectorSegment::Param),
            ast::Value::SingleQuotedString(s)
            | ast::Value::DoubleQuotedString(s)
            | ast::Value::EscapedStringLiteral(s) => Some(JsonSelectorSegment::Literal(s.clone())),
            ast::Value::Number(n, _) => Some(JsonSelectorSegment::Literal(n.to_string())),
            _ => None,
        }
    }

    /// The [`ast::Value`] an expression ultimately is, seeing through casts
    /// (`$1::jsonb`, `'a'::text`). Casts are common on both halves — the client
    /// may write them and earlier rules may add them.
    fn as_ast_value(expr: &'ast Expr) -> Option<&'ast ast::Value> {
        match expr {
            Expr::Value(value_with_span) => Some(&value_with_span.value),
            Expr::Cast { expr, .. } => Self::as_ast_value(expr),
            _ => None,
        }
    }
}
