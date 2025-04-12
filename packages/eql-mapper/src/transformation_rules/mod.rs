//! Transformation rules

mod helpers;

mod eql_col_in_projection_and_group_by;
mod replace_plaintext_eql_literals;
mod order_by_expr_with_eql_type;
mod group_by_eql_col;
mod preserve_aliases;
mod use_equivalent_eql_fns_on_eql_types;

pub(self) mod selector;

pub(crate) use eql_col_in_projection_and_group_by::*;
pub(crate) use replace_plaintext_eql_literals::*;
pub(crate) use order_by_expr_with_eql_type::*;
pub(crate) use group_by_eql_col::*;
pub(crate) use preserve_aliases::*;
pub(crate) use use_equivalent_eql_fns_on_eql_types::*;

use crate::EqlMapperError;
use selector::Selector;
use sqltk::{Context, Visitable};

pub(crate) trait Rule<'ast> {
    type Sel: Selector;

    fn apply<'ast_new: 'ast, N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N0,
        target_node: &'ast_new mut N0,
    ) -> Result<(), EqlMapperError>;
}