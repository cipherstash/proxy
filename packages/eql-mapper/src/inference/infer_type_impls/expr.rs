use sqlparser::ast::{BinaryOperator, Expr};

use crate::{inference::unifier::Type, inference::InferType, inference::TypeError, TypeInferencer};

impl<'ast> InferType<'ast, Expr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, this_expr: &'ast Expr) -> Result<(), TypeError> {
        match this_expr {
            Expr::Identifier(ident) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    self.scope.borrow().resolve_ident(ident)?,
                )?;
            }

            Expr::CompoundIdentifier(idents) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    self.scope.borrow().resolve_compound_ident(idents)?,
                )?;
            }

            Expr::Wildcard => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    self.scope.borrow().resolve_wildcard()?,
                )?;
            }

            Expr::QualifiedWildcard(object_name) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    self.scope
                        .borrow()
                        .resolve_qualified_wildcard(&object_name.0)?,
                )?;
            }

            Expr::JsonAccess { .. } => {
                return Err(TypeError::UnsupportedSqlFeature(
                    "Snowflake-style JSON access".into(),
                ))
            }

            Expr::CompositeAccess { expr, key: _ }
            | Expr::IsFalse(expr)
            | Expr::IsNotFalse(expr)
            | Expr::IsTrue(expr)
            | Expr::IsNotTrue(expr)
            | Expr::IsNull(expr)
            | Expr::IsNotNull(expr)
            | Expr::IsUnknown(expr)
            | Expr::IsNotUnknown(expr) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
            }

            Expr::IsDistinctFrom(a, b) | Expr::IsNotDistinctFrom(a, b) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.unify_and_log(this_expr, self.get_type(&**a), self.get_type(&**b))?,
                    Type::fresh_tvar(),
                )?;
            }

            Expr::InList {
                expr,
                list,
                negated: _,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**expr),
                    list.iter().try_fold(self.get_type(&**expr), |a, b| {
                        self.unify_and_log(this_expr, a, self.get_type(b))
                    })?,
                )?;
            }

            Expr::InSubquery {
                expr,
                subquery,
                negated: _,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::fresh_tvar())?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**subquery),
                    Type::projection(&[(self.get_type(&**expr), None)]),
                )?;
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
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.unify_and_log(this_expr, self.get_type(&**expr), self.get_type(&**low))?,
                    self.get_type(&**high),
                )?;
            }

            Expr::BinaryOp { left, op, right } => {
                match op {
                    BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo
                    | BinaryOperator::StringConcat
                    | BinaryOperator::Gt
                    | BinaryOperator::Lt
                    | BinaryOperator::GtEq
                    | BinaryOperator::LtEq
                    | BinaryOperator::Spaceship
                    | BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::And
                    | BinaryOperator::Or
                    | BinaryOperator::Xor
                    | BinaryOperator::BitwiseOr
                    | BinaryOperator::BitwiseAnd
                    | BinaryOperator::BitwiseXor
                    | BinaryOperator::DuckIntegerDivide
                    | BinaryOperator::MyIntegerDivide
                    | BinaryOperator::Custom(_)
                    | BinaryOperator::PGBitwiseXor
                    | BinaryOperator::PGBitwiseShiftLeft
                    | BinaryOperator::PGBitwiseShiftRight
                    | BinaryOperator::PGExp
                    | BinaryOperator::PGOverlap
                    | BinaryOperator::PGRegexMatch
                    | BinaryOperator::PGRegexIMatch
                    | BinaryOperator::PGRegexNotMatch
                    | BinaryOperator::PGRegexNotIMatch
                    | BinaryOperator::PGLikeMatch
                    | BinaryOperator::PGILikeMatch
                    | BinaryOperator::PGNotLikeMatch
                    | BinaryOperator::PGNotILikeMatch
                    | BinaryOperator::PGStartsWith
                    | BinaryOperator::PGCustomBinaryOperator(_) => {
                        self.unify_and_log(
                            this_expr,
                            self.get_type(this_expr),
                            self.unify(self.get_type(&**left), self.get_type(&**right))?,
                        )?;
                    }

                    // JSON(B) operators.
                    // Left side is JSON(B) and must unify to Scalar::Native, or Scalar::Encrypted(_).
                    BinaryOperator::Arrow
                    | BinaryOperator::LongArrow
                    | BinaryOperator::HashArrow
                    | BinaryOperator::HashLongArrow
                    | BinaryOperator::AtAt
                    | BinaryOperator::HashMinus
                    | BinaryOperator::AtQuestion
                    | BinaryOperator::Question
                    | BinaryOperator::QuestionAnd
                    | BinaryOperator::QuestionPipe => {
                        self.unify_and_log(
                            this_expr,
                            self.get_type(this_expr),
                            self.unify_and_log(
                                left,
                                self.get_type(&**left),
                                self.get_type(&**right),
                            )?,
                        )?;
                    }

                    // JSON(B)/Array containment operators (@> and <@)
                    // Both sides must unify to the same type.
                    BinaryOperator::AtArrow | BinaryOperator::ArrowAt => {
                        self.unify_and_log(
                            this_expr,
                            self.get_type(this_expr),
                            self.unify_and_log(
                                left,
                                self.get_type(&**left),
                                self.get_type(&**right),
                            )?,
                        )?;
                    }
                }
            }

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
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), self.get_type(&**pattern))?;
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
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.unify_and_log(
                        this_expr,
                        self.get_type(&**expr),
                        self.get_type(&**pattern),
                    )?,
                    Type::anonymous_native(),
                )?;
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
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**left), self.get_type(&**right))?;
            }

            Expr::Ceil { expr, .. }
            | Expr::Floor { expr, .. }
            | Expr::UnaryOp { expr, .. }
            | Expr::Convert { expr, .. }
            | Expr::Cast { expr, .. } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
            }

            Expr::AtTimeZone {
                timestamp,
                time_zone,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**time_zone),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**timestamp),
                    Type::anonymous_native(),
                )?;
            }

            Expr::Extract {
                field: _,
                syntax: _,
                expr,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
            }

            Expr::Position { expr, r#in } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.unify_and_log(this_expr, self.get_type(&**expr), self.get_type(&**r#in))?,
                    Type::anonymous_native(),
                )?;
            }

            Expr::Substring {
                expr,
                substring_from,
                substring_for,
                special: _,
            } => {
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                if let Some(expr) = substring_from {
                    self.unify_and_log(
                        this_expr,
                        self.get_type(&**expr),
                        Type::anonymous_native(),
                    )?;
                }
                if let Some(expr) = substring_for {
                    self.unify_and_log(
                        this_expr,
                        self.get_type(&**expr),
                        Type::anonymous_native(),
                    )?;
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
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
                if let Some(trim_where) = trim_where {
                    self.unify_and_log(
                        this_expr,
                        self.get_type(trim_where),
                        Type::anonymous_native(),
                    )?;
                }
                if let Some(trim_what) = trim_what {
                    self.unify_and_log(
                        this_expr,
                        self.get_type(&**trim_what),
                        Type::anonymous_native(),
                    )?;
                }
                if let Some(trim_characters) = trim_characters {
                    trim_characters
                        .iter()
                        .try_fold(Type::anonymous_native(), |a, b| {
                            self.unify_and_log(this_expr, a, self.get_type(b))
                        })?;
                }
            }

            Expr::Overlay {
                expr,
                overlay_what,
                overlay_from,
                overlay_for,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**overlay_what),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**overlay_from),
                    Type::anonymous_native(),
                )?;
                if let Some(overlay_for) = overlay_for {
                    self.unify_and_log(
                        this_expr,
                        self.get_type(&**overlay_for),
                        Type::anonymous_native(),
                    )?;
                }
            }

            Expr::Collate { expr, collation: _ } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(this_expr, self.get_type(&**expr), Type::anonymous_native())?;
            }

            // The current `Expr` shares the same type hole as the sub-expression
            Expr::Nested(expr) => {
                self.unify_and_log(this_expr, self.get_type(this_expr), self.get_type(&**expr))?;
            }

            // The current `Expr` shares the same type hole as the `Value`.
            Expr::Value(_) => {
                // self.unify_and_log(this_expr, self.get_type(this_expr), self.get_type(value))?;
                self.get_type(this_expr);
            }

            Expr::IntroducedString { .. } => Err(TypeError::UnsupportedSqlFeature(
                "MySQL charset introducer".into(),
            ))?,

            Expr::TypedString {
                data_type: _,
                value: _,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
            }

            Expr::MapAccess { column: _, keys: _ } => Err(TypeError::UnsupportedSqlFeature(
                "ClickHouse-style map access".into(),
            ))?,

            // The return type of this function and the return type of this statement must be the same type.
            Expr::Function(function) => {
                self.unify_and_log(this_expr, self.get_type(this_expr), self.get_type(function))?;
            }

            // When operand is Some(operand), all conditions must be of type expr and expr must support equality
            // When operand is None, all conditions must be native (they are boolean)
            // The elements of `results` and else_result must be the same type
            // The type of the overall expression is the type of the results/else_result
            Expr::Case {
                operand,
                conditions,
                results,
                else_result,
            } => {
                match operand {
                    Some(operand) => {
                        let condition_ty = Type::fresh_tvar();
                        self.unify_and_log(
                            this_expr,
                            self.get_type(&**operand),
                            conditions.iter().try_fold(condition_ty, |a, b| {
                                self.unify_and_log(this_expr, a, self.get_type(b))
                            })?,
                        )?;
                    }
                    None => {
                        self.unify_and_log(
                            this_expr,
                            Type::anonymous_native(),
                            conditions
                                .iter()
                                .try_fold(Type::anonymous_native(), |a, b| {
                                    self.unify_and_log(this_expr, a, self.get_type(b))
                                })?,
                        )?;
                    }
                }

                let results_type = results.iter().try_fold(Type::fresh_tvar(), |a, b| {
                    self.unify_and_log(this_expr, a, self.get_type(b))
                })?;

                if let Some(else_result) = else_result {
                    self.unify_and_log(
                        this_expr,
                        results_type.clone(),
                        self.get_type(&**else_result),
                    )?;
                };

                self.unify_and_log(this_expr, self.get_type(this_expr), results_type)?;
            }

            Expr::Exists {
                subquery: _,
                negated: _,
            } => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
            }

            Expr::Subquery(subquery) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    self.get_type(&**subquery),
                )?;
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

            // This is an array element access by index.
            // `expr` must be an array
            // `self.get_type(this_expr)` must be the same as the array element type
            Expr::Subscript { expr, subscript: _ } => {
                let elem_type = Type::fresh_tvar();
                self.unify_and_log(
                    this_expr,
                    self.get_type(&**expr),
                    Type::array(elem_type.clone()),
                )?;
                self.unify_and_log(this_expr, self.get_type(this_expr), elem_type)?;
            }

            Expr::Array(array) => {
                let array_ty = Type::array(
                    // Constrain all elements of the array to be the same type.
                    array.elem.iter().try_fold(Type::fresh_tvar(), |a, b| {
                        self.unify_and_log(this_expr, a, self.get_type(b))
                    })?,
                );
                self.unify_and_log(this_expr, self.get_type(this_expr), array_ty)?;
            }

            // interval is unmapped, value is unmapped
            Expr::Interval(interval) => {
                self.unify_and_log(
                    this_expr,
                    self.get_type(this_expr),
                    Type::anonymous_native(),
                )?;
                self.unify_and_log(
                    this_expr,
                    self.get_type(&*interval.value),
                    Type::anonymous_native(),
                )?;
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
        }

        Ok(())
    }
}
