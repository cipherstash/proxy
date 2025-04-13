//! Transformation rules

mod helpers;

mod wrap_grouped_eql_col_in_aggregate_fn;
mod fail_on_placeholder_change;
mod group_by_eql_col;
mod wrap_eql_cols_in_order_by_with_ore_fn;
mod preserve_effective_aliases;
mod replace_plaintext_eql_literals;
mod use_equivalent_eql_fns_on_eql_types;

pub(crate) use wrap_grouped_eql_col_in_aggregate_fn::*;
pub(crate) use fail_on_placeholder_change::*;
pub(crate) use group_by_eql_col::*;
pub(crate) use wrap_eql_cols_in_order_by_with_ore_fn::*;
pub(crate) use preserve_effective_aliases::*;
pub(crate) use replace_plaintext_eql_literals::*;
pub(crate) use use_equivalent_eql_fns_on_eql_types::*;

use crate::EqlMapperError;
use sqltk::{NodePath, Visitable};

use impl_trait_for_tuples::*;

/// This trait is essentially the same as [`sqltk::Transform`] but it operates on an `&mut N` instead of an owned `N`.
///
/// Owned values cannot be downcasted, which is why this trait exists.
pub(crate) trait TransformationRule<'ast> {
    /// If the rule is applicable to the `node_path` and `target_node` then implementations should mutate `target_node` accordingly and return `Ok(())`.
    ///
    /// If the rule is not applicable then simply return `Ok(())` without mutating `target_node`.
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError>;

    /// Some transformations have one or more internal invariants that must hold after the overall AST transformation has completed.
    ///
    /// The default implementation returns `Ok(())`.
    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        Ok(())
    }
}

#[impl_for_tuples(1, 16)]
impl<'ast> TransformationRule<'ast> for Tuple {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError> {
        for_tuples!( #(Tuple.apply(node_path, target_node)?; )* );
        Ok(())
    }

    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        for_tuples!( #(Tuple.check_postcondition()?; )* );

        Ok(())
    }
}
