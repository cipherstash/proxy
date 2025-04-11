//! Transformation rules

mod helpers;

mod eql_col_in_projection_and_group_by;
mod group_by_eql_cols;
mod preserve_aliases;
mod use_equivalent_eql_fns_on_eql_types;

pub(self) mod selector;

pub(crate) use eql_col_in_projection_and_group_by::*;
pub(crate) use group_by_eql_cols::*;
pub(crate) use preserve_aliases::*;
pub(crate) use use_equivalent_eql_fns_on_eql_types::*;

use crate::EqlMapperError;
use selector::Selector;
use sqltk::{Context, Visitable};

pub(crate) trait Rule<'ast> {
    type Sel: Selector;

    fn apply<N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        original_node: &'ast N0,
        target_node: &mut N0,
    ) -> Result<(), EqlMapperError>;
}