//! The N:1 fusion record for encrypted-JSON equality.
//!
//! `col -> sel = value` does not compare two encrypted terms. Exact JSON
//! equality in EQL v3 is *containment of a value selector*: a single keyed MAC
//! over the path and the canonicalised value together
//! (`QueryOp::SteVecValueSelector`, input `{"path": <jsonpath>, "value":
//! <scalar>}`). One needle, built from **two** SQL operands.
//!
//! The mapper cannot build it — it holds no encryption key. So the mapper does
//! the half it can: it types the value operand [`EqlTerm::JsonValueSelector`],
//! drops the path operand from the rewritten SQL, and records *where the path
//! came from* so the proxy can fuse the pair at encryption time.
//!
//! [`EqlTerm::JsonValueSelector`]: crate::EqlTerm::JsonValueSelector

use std::collections::HashMap;
use std::marker::PhantomData;

use sqltk::parser::ast::{self};
use sqltk::parser::ast::{
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectNamePart,
};
use sqltk::NodeKey;

use crate::Param;

/// One step of the path half of a fused value selector.
///
/// A step is independently a literal or a placeholder, so all combinations
/// occur (`-> 'a' = '1'`, `-> $1 = $2`, `-> 'a' -> $1 = $2`, …). A literal step
/// is fully known at type-check time and is carried inline; a placeholder step
/// is only known at Bind, so its param number is carried instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonSelectorSegment {
    /// A SQL literal selector (`col -> 'name' = …`) — the selector text itself.
    Literal(String),

    /// A placeholder selector (`col -> $1 = …`) — the param it arrives in.
    Param(Param),
}

/// Where the JSON path half of a fused value selector comes from.
///
/// The path is a **sequence** of steps, because an accessor chain is a single
/// path: `col -> 'a' -> 'b' = value` selects `$.a.b` of the whole document, not
/// `$.b` of some intermediate one. Nothing between the column and the value is
/// a jsonb value the database could operate on — the payload is encrypted — so
/// the whole chain has to collapse into one path, composed here and resolved to
/// text by the proxy once every placeholder step is bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonSelectorSource {
    segments: Vec<JsonSelectorSegment>,
}

impl JsonSelectorSource {
    pub(crate) fn new(segments: Vec<JsonSelectorSegment>) -> Self {
        Self { segments }
    }

    /// A single-step literal path.
    pub fn literal(path: impl Into<String>) -> Self {
        Self::new(vec![JsonSelectorSegment::Literal(path.into())])
    }

    /// A single-step placeholder path.
    pub fn param(param: Param) -> Self {
        Self::new(vec![JsonSelectorSegment::Param(param)])
    }

    /// The steps of the path, outermost accessor last: `col -> 'a' -> 'b'` is
    /// `["a", "b"]`, which composes to `$.a.b`.
    pub fn segments(&self) -> &[JsonSelectorSegment] {
        &self.segments
    }

    /// Every input param the path consumes.
    pub fn params(&self) -> impl Iterator<Item = Param> + '_ {
        self.segments.iter().filter_map(|segment| match segment {
            JsonSelectorSegment::Param(param) => Some(*param),
            JsonSelectorSegment::Literal(_) => None,
        })
    }
}

/// Decomposes a JSON field-access chain into the expression it is rooted at and
/// the selectors applied to it, outermost last.
///
/// Recognises both spellings at every step, so a chain may mix them:
///
/// - `col -> sel`, `col ->> sel` (and the `eql_v3."->"(col, sel)` form the
///   containment rule rewrites them to)
/// - `jsonb_path_query_first(col, sel)`, `jsonb_path_query(col, sel)`
///
/// Returns `None` for anything that is not a field access — importantly for a
/// bare column, which is a whole document rather than a field of one.
///
/// Parentheses are transparent at every position. `(col -> 'a') -> 'b'` is the
/// same query as `col -> 'a' -> 'b'` and must decompose to the same root and the
/// same path: a chain the walker fails to see through is not merely unfused, it
/// is a leak. The unseen inner accessor gets treated as the root container, so
/// its selector reaches PostgreSQL in cleartext and native jsonb `->` is applied
/// to an encrypted payload.
pub(crate) fn json_accessor_chain(expr: &Expr) -> Option<(&Expr, Vec<&Expr>)> {
    let mut selectors = Vec::new();
    let mut container = unnest(expr);

    while let Some((inner, selector)) = json_accessor(container) {
        selectors.push(unnest(selector));
        container = unnest(inner);
    }

    if selectors.is_empty() {
        return None;
    }

    selectors.reverse();

    Some((container, selectors))
}

/// Strips redundant parentheses, however many deep.
///
/// `Expr::Nested` carries no meaning of its own — it records that the author
/// wrote brackets. Every consumer of a chain wants the expression inside them.
pub(crate) fn unnest(expr: &Expr) -> &Expr {
    let mut current = expr;

    while let Expr::Nested(inner) = current {
        current = inner;
    }

    current
}

/// One step of a field access: `(container, selector)`.
pub(crate) fn json_accessor(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Arrow | BinaryOperator::LongArrow,
            right,
        } => Some((&**left, &**right)),

        Expr::Function(function) if is_json_accessor_fn(&function.name) => match &function.args {
            FunctionArguments::List(list) => match list.args.as_slice() {
                [FunctionArg::Unnamed(FunctionArgExpr::Expr(container)), FunctionArg::Unnamed(FunctionArgExpr::Expr(selector))] => {
                    Some((container, selector))
                }
                _ => None,
            },
            _ => None,
        },

        _ => None,
    }
}

/// Whether a function call is a JSON field access.
///
/// Matched on the bare function name, so every schema spelling counts — the
/// client's `pg_catalog.jsonb_path_query_first`, the `eql_v3.` twin, and the
/// `eql_v3."->"` form the containment rewrite produces. Names outside this set
/// are NOT accessors: an arbitrary two-argument call over an encrypted JSON
/// value (`coalesce(a, b)`) would otherwise be mistaken for one and its second
/// argument read as a selector.
fn is_json_accessor_fn(name: &ast::ObjectName) -> bool {
    let Some(ObjectNamePart::Identifier(ident)) = name.0.last() else {
        return false;
    };

    matches!(
        ident.value.to_lowercase().as_str(),
        "jsonb_path_query" | "jsonb_path_query_first" | "->" | "->>"
    )
}

/// A second, *different* path recorded against the same operand.
///
/// The map is keyed by operand, so one param used as the outermost selector of
/// two chains with different prefixes (`SELECT j -> 'a' -> $1, j -> 'b' -> $1`)
/// would silently overwrite one path with the other and answer one of the two
/// projections from the wrong field. There is no key that could tell them apart:
/// at Bind time the proxy has only the param number.
#[derive(Debug)]
pub struct ConflictingSelectorPath;

/// Role marker: the path half of a fused equality needle, which the proxy MACs
/// together with a value (`QueryOp::SteVecValueSelector`).
#[derive(Debug, Default)]
pub struct FusedValue;

/// Role marker: the composed path of a collapsed accessor chain, which the proxy
/// MACs on its own (`QueryOp::SteVecSelector`).
#[derive(Debug, Default)]
pub struct AccessorChain;

/// For each operand that carries a JSON selector, where the *path* it keys comes
/// from.
///
/// Keyed separately for the two protocols the proxy has to serve — params are
/// addressed by number (the extended protocol has no AST at Bind time),
/// literals by AST node.
#[derive(Debug, Default)]
pub struct JsonSelectorSources<'ast, Role> {
    by_param: HashMap<Param, JsonSelectorSource>,
    by_literal: HashMap<NodeKey<'ast>, JsonSelectorSource>,
    _role: PhantomData<Role>,
}

/// The fused JSON value selectors in a statement: for each operand that carries
/// the *value* half of an exact JSON equality, where its *path* half comes from.
///
/// The path operand is dropped from the rewritten SQL — the proxy MACs path and
/// value together into one needle — so this is the only record of it.
pub type JsonValueSelectors<'ast> = JsonSelectorSources<'ast, FusedValue>;

/// The composed paths of collapsed accessor chains: for each *selector* operand
/// that survives a multi-step chain, every step of the path it must key.
///
/// `j -> 'a' -> 'b'` emits the single accessor `eql_v3."->"(j, <sel>)`, where
/// `<sel>` is the outermost selector operand and the path it keys is `$.a.b` —
/// the whole chain, not the one segment its own text spells. The inner
/// accessors are dropped from the SQL, so without this record the proxy would
/// key `$.b` and select nothing.
///
/// Distinct from [`JsonValueSelectors`] because the two produce different
/// encryption ops: a bare selector (`QueryOp::SteVecSelector`) against a
/// path-and-value needle (`QueryOp::SteVecValueSelector`).
pub type JsonAccessorPaths<'ast> = JsonSelectorSources<'ast, AccessorChain>;

impl<'ast, Role> JsonSelectorSources<'ast, Role> {
    /// Records the path for an operand arriving in `param`.
    ///
    /// Recording the *same* path twice is fine — the same param may legitimately
    /// select the same path in several places. A different one is not: see
    /// [`ConflictingSelectorPath`].
    pub(crate) fn record_param(
        &mut self,
        param: Param,
        source: JsonSelectorSource,
    ) -> Result<(), ConflictingSelectorPath> {
        match self.by_param.get(&param) {
            Some(existing) if *existing != source => Err(ConflictingSelectorPath),
            _ => {
                self.by_param.insert(param, source);
                Ok(())
            }
        }
    }

    /// Records the path for a literal operand.
    ///
    /// Unlike a param, a literal is keyed by AST *node* identity, so two
    /// occurrences of the same text are distinct keys and cannot conflict.
    pub(crate) fn record_literal(&mut self, node: &'ast ast::Value, source: JsonSelectorSource) {
        self.by_literal.insert(NodeKey::new(node), source);
    }

    /// The path source for the operand bound to `param`, or `None` if that param
    /// is not one.
    pub fn for_param(&self, param: Param) -> Option<&JsonSelectorSource> {
        self.by_param.get(&param)
    }

    /// The path source for the operand at literal `node`, or `None` if that
    /// literal is not one.
    pub fn for_literal(&self, node: &'ast ast::Value) -> Option<&JsonSelectorSource> {
        self.by_literal.get(&NodeKey::new(node))
    }

    pub fn is_empty(&self) -> bool {
        self.by_param.is_empty() && self.by_literal.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::json_accessor_chain;
    use sqltk::parser::{dialect::PostgreSqlDialect, parser::Parser};

    /// `(root, selectors)` of the chain in `sql`, rendered as SQL text.
    fn chain_of(sql: &str) -> Option<(String, Vec<String>)> {
        let expr = Parser::new(&PostgreSqlDialect {})
            .try_with_sql(sql)
            .unwrap()
            .parse_expr()
            .unwrap();

        json_accessor_chain(&expr).map(|(container, selectors)| {
            (
                container.to_string(),
                selectors.iter().map(|s| s.to_string()).collect(),
            )
        })
    }

    #[test]
    fn a_chain_yields_its_root_and_every_selector_in_order() {
        assert_eq!(
            chain_of("j -> 'a' -> 'b' -> 'c'"),
            Some((
                "j".to_owned(),
                vec!["'a'".to_owned(), "'b'".to_owned(), "'c'".to_owned()]
            ))
        );
    }

    #[test]
    fn a_chain_may_mix_spellings() {
        assert_eq!(
            chain_of("jsonb_path_query_first(j, '$.a') ->> $1"),
            Some(("j".to_owned(), vec!["'$.a'".to_owned(), "$1".to_owned()]))
        );
    }

    #[test]
    fn a_bare_column_is_not_a_field_access() {
        assert_eq!(chain_of("j"), None);
    }

    /// A two-argument call that is not an accessor must stop the walk, or its
    /// second argument would be read as a selector and the call itself stripped
    /// from the statement.
    #[test]
    fn a_non_accessor_call_is_the_root_not_a_step() {
        assert_eq!(
            chain_of("coalesce(j, k) -> 'a'"),
            Some(("coalesce(j, k)".to_owned(), vec!["'a'".to_owned()]))
        );
        assert_eq!(chain_of("coalesce(j, k)"), None);
    }
}
