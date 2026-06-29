use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{
    BinaryOperator, CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr,
    FunctionArgumentList, FunctionArguments, Ident, ObjectName, ObjectNamePart, OrderByExpr,
    ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::ste_vec_ordering::{is_eql_typed, is_ste_vec_accessor};
use crate::unifier::{Type, Value};
use crate::{EqlMapperError, IndexKind, IndexResolver, TableColumn};

use super::TransformationRule;

/// Rewrites ordering comparisons (`<`, `<=`, `>`, `>=`) and `ORDER BY` sort
/// keys on a *scalar* (non-jsonb) `eql_v2_encrypted` column whose concrete
/// index set contains `Ope` (order-preserving encryption) so that PostgreSQL
/// compares the order-preserving `op` ciphertext directly using built-ins only.
///
/// # Why this is needed
///
/// An OPE-indexed column stores an order-preserving ciphertext in the `op` slot
/// of its EQL payload. Because OPE preserves order under byte (memcmp)
/// comparison, ordering can be evaluated entirely by PostgreSQL built-ins —
/// `decode(... ->> 'op', 'hex')` extracts the hex-encoded `op` bytea, and
/// `bytea` comparison is exactly the OPE order. No EQL function is required.
///
/// This rule rewrites:
///
/// ```sql
/// col <op> $param
/// ```
///
/// into:
///
/// ```sql
/// decode(col ->> 'op', 'hex') <op> decode($param ->> 'op', 'hex')
/// ```
///
/// and:
///
/// ```sql
/// ORDER BY col [ASC|DESC] [NULLS …]
/// ```
///
/// into:
///
/// ```sql
/// ORDER BY decode(col ->> 'op', 'hex') [ASC|DESC] [NULLS …]
/// ```
///
/// # What is deliberately left alone
///
/// - **ORE columns** (`Ore` present, `Ope` absent): they keep the existing
///   bare-operator root Block-ORE (`ob`) path.
/// - **Equality** (`=` / `<>`): a separate concern (handled elsewhere).
/// - **jsonb STE-vec accessors** (`col -> selector`): handled by
///   [`super::RewriteJsonbSteVecOrdering`]; this rule only matches *bare*
///   scalar EQL column references.
/// - **Columns with no concrete index info** (empty resolver): no rewrite,
///   preserving the pre-resolver behaviour.
///
/// The right-hand-side param/literal node is moved *intact* into the new
/// `decode(...)` wrapper. ([`mem::replace`] is just the mechanism for taking
/// ownership of the boxed operand out of the `&mut` binding — it leaves the
/// original AST node untouched, not a copy.) Preserving that node means the
/// downstream [`super::CastParamsAsEncrypted`] / [`super::CastLiteralsAsEncrypted`]
/// rules still recognise and cast it to `::JSONB::eql_v2_encrypted`, yielding an
/// EQL payload that carries its own `op` term for the same column.
#[derive(Debug)]
pub struct RewriteScalarOpeOrdering<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
    index_resolver: Arc<dyn IndexResolver>,
}

impl<'ast> RewriteScalarOpeOrdering<'ast> {
    pub fn new(
        node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
        index_resolver: Arc<dyn IndexResolver>,
    ) -> Self {
        Self {
            node_types,
            index_resolver,
        }
    }

    /// Returns the [`TableColumn`] of `expr` if it is an EQL-typed node, else `None`.
    fn eql_table_column(&self, expr: &Expr) -> Option<TableColumn> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => Some(eql_term.table_column().clone()),
            _ => None,
        }
    }

    /// Returns `true` if `expr` is a *scalar* OPE-indexed EQL column reference.
    ///
    /// "Scalar" excludes jsonb STE-vec accessors (`->` / `->>` /
    /// `jsonb_path_query_first`), which are handled by the jsonb ordering rule.
    fn is_scalar_ope_column(&self, expr: &Expr) -> bool {
        if is_ste_vec_accessor(expr) {
            return false;
        }

        match self.eql_table_column(expr) {
            Some(table_column) => self
                .index_resolver
                .resolve(&table_column)
                .contains(&IndexKind::Ope),
            None => false,
        }
    }

    /// Wraps `expr` in `decode((<expr>)::jsonb ->> 'op', 'hex')`.
    ///
    /// `(<expr>)::jsonb ->> 'op'` extracts the order-preserving ciphertext as
    /// `text`, and `decode(…, 'hex')` turns it into the `bytea` whose memcmp
    /// order is the OPE order.
    ///
    /// The `::jsonb` cast is REQUIRED. `eql_v2_encrypted` defines its own `->>`
    /// operator (a selector accessor, alias of `->`, that treats the right
    /// operand as an encrypted selector), so `col ->> 'op'` on the encrypted
    /// domain binds to that and misfires (`malformed record literal: "op"`).
    /// Casting to the base `jsonb` type forces native jsonb field extraction.
    /// A binary operand is wrapped in parentheses first so the cast applies to
    /// the whole operand rather than binding tighter than `->` to its right
    /// child.
    fn decode_op(expr: Expr) -> Expr {
        let expr = match expr {
            binary @ Expr::BinaryOp { .. } => Expr::Nested(Box::new(binary)),
            other => other,
        };

        let expr = Expr::Cast {
            kind: CastKind::DoubleColon,
            expr: Box::new(expr),
            data_type: DataType::JSONB,
            format: None,
        };

        let extract_op = Expr::BinaryOp {
            left: Box::new(expr),
            op: BinaryOperator::LongArrow, // ->>
            right: Box::new(Expr::Value(ValueWithSpan {
                value: SqltkValue::SingleQuotedString("op".into()),
                span: Span::empty(),
            })),
        };

        Expr::Function(Function {
            name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("decode"))]),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(extract_op)),
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Value(ValueWithSpan {
                        value: SqltkValue::SingleQuotedString("hex".into()),
                        span: Span::empty(),
                    }))),
                ],
                duplicate_treatment: None,
                clauses: vec![],
            }),
            parameters: FunctionArguments::None,
            filter: None,
            null_treatment: None,
            over: None,
            within_group: vec![],
        })
    }

    fn dummy_expr() -> Expr {
        Expr::Value(ValueWithSpan {
            value: SqltkValue::Null,
            span: Span::empty(),
        })
    }

    /// Returns `true` if the binary expression at the head of `node_path` is a
    /// scalar OPE ordering comparison that this rule would rewrite.
    fn would_edit_comparison(&self, node_path: &NodePath<'ast>) -> bool {
        if let Some((Expr::BinaryOp { left, op, right },)) = node_path.last_1_as::<Expr>() {
            return matches!(
                op,
                BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
            ) && self.is_scalar_ope_column(left)
                && is_eql_typed(&self.node_types, right);
        }
        false
    }

    /// Returns `true` if the `Expr` at the head of `node_path` is the sort key
    /// of an `ORDER BY` clause and is a scalar OPE column.
    fn would_edit_order_by(&self, node_path: &NodePath<'ast>) -> bool {
        if let Some((_order_by, expr)) = node_path.last_2_as::<OrderByExpr, Expr>() {
            return self.is_scalar_ope_column(expr);
        }
        false
    }
}

impl<'ast> TransformationRule<'ast> for RewriteScalarOpeOrdering<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Case 1: `col <op> $param` comparison — rewrite the whole BinaryOp.
        if self.would_edit_comparison(node_path) {
            let expr = target_node.downcast_mut::<Expr>().unwrap();
            if let Expr::BinaryOp { left, op: _, right } = expr {
                let left_expr = mem::replace(&mut **left, Self::dummy_expr());
                let right_expr = mem::replace(&mut **right, Self::dummy_expr());

                **left = Self::decode_op(left_expr);
                **right = Self::decode_op(right_expr);
                return Ok(true);
            }
        }

        // Case 2: `ORDER BY col` sort key — rewrite just the sort-key Expr.
        if self.would_edit_order_by(node_path) {
            let expr = target_node.downcast_mut::<Expr>().unwrap();
            let inner = mem::replace(expr, Self::dummy_expr());
            *expr = Self::decode_op(inner);
            return Ok(true);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        self.would_edit_comparison(node_path) || self.would_edit_order_by(node_path)
    }
}
