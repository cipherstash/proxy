//! Transformation rules

mod helpers;

mod eql_col_in_projection_and_group_by;
mod group_by_eql_col;
mod order_by_expr_with_eql_type;
mod preserve_aliases;
mod fail_on_placeholder_change;
mod replace_plaintext_eql_literals;
mod use_equivalent_eql_fns_on_eql_types;

pub(self) mod selector;

pub(crate) use eql_col_in_projection_and_group_by::*;
pub(crate) use group_by_eql_col::*;
pub(crate) use order_by_expr_with_eql_type::*;
pub(crate) use preserve_aliases::*;
pub(crate) use fail_on_placeholder_change::*;
pub(crate) use replace_plaintext_eql_literals::*;
pub(crate) use use_equivalent_eql_fns_on_eql_types::*;

use crate::EqlMapperError;
use sqltk::{Context, Visitable};

use impl_trait_for_tuples::*;

pub(crate) trait Rule<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError>;
}

#[impl_for_tuples(1, 16)]
impl<'ast> Rule<'ast> for Tuple {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError> {
        for_tuples!( #(let target_node = Tuple.apply(ctx, source_node, target_node)?; )*);
        Ok(target_node)
    }
}