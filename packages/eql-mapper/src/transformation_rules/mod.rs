//! # Transformation rules
//!
//! - [`TransformationRule`] is a trait for implementing modular AST transformation rules which can be composed into
//!   one rule.
//!
//! - [`DryRun`] is a type for checking if a `TransformationRule` will mutate the AST without actually mutating the AST.
//!   It is useful as a performance optimisation to avoid rebuilding an AST if no changes are required.
//!
//! This module implements `TransformationRule` for tuples of size 1 to 16 where all of their elements implement
//! `TransformationRule`. This facilitates composition of rules into single rules.

mod helpers;

mod fail_on_placeholder_change;
mod group_by_eql_col;
mod preserve_effective_aliases;
mod replace_plaintext_eql_literals;
mod rewrite_standard_sql_fns_on_eql_types;
mod wrap_eql_cols_in_order_by_with_ore_fn;
mod wrap_eql_params_in_row;
mod wrap_grouped_eql_col_in_aggregate_fn;

use std::marker::PhantomData;

pub(crate) use fail_on_placeholder_change::*;
pub(crate) use group_by_eql_col::*;
pub(crate) use preserve_effective_aliases::*;
pub(crate) use replace_plaintext_eql_literals::*;
pub(crate) use rewrite_standard_sql_fns_on_eql_types::*;
pub(crate) use wrap_eql_cols_in_order_by_with_ore_fn::*;
pub(crate) use wrap_eql_params_in_row::*;
pub(crate) use wrap_grouped_eql_col_in_aggregate_fn::*;

use crate::EqlMapperError;
use sqltk::{NodePath, Transform, Visitable};

use impl_trait_for_tuples::*;

/// This trait is essentially the same as [`sqltk::Transform`] but it operates on an `&mut N` instead of an owned `N`.
///
/// Owned values cannot be downcasted, which is why this trait exists.
pub(crate) trait TransformationRule<'ast> {
    /// If the rule is applicable to the `node_path` and `target_node` then `target_node` will be mutated accordingly.
    ///
    /// Returns `Ok(true)` if changes are made to the AST, `Ok(false)` otherwise.
    ///
    /// If the invariants become violated then this method will return `Err(err)`.
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError>;

    /// Tests if the rule would modify the AST.
    ///
    /// The intent is that implementors of this method will execute all of the same logic as
    /// [`TransformationRule::apply`] *except* for the actual modification of the AST - and return `true` if an AST
    /// modification would occur.
    ///
    /// If any part of the implementation fails with an error before it can be determined if an edit would occur then
    /// `would_edit` **MUST** return `true` to force the caller to attempt transformation which will propagate the
    /// error.  Otherwise the caller may end up incorrectly skipping transformation.
    #[allow(unused)]
    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, target_node: &N) -> bool {
        true
    }

    /// Some transformations have one or more internal invariants that must hold after the overall AST transformation
    /// has completed.
    ///
    /// The default implementation returns `Ok(())`.
    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        Ok(())
    }
}

/// A [`TransformationRule`] with two modes: one for testing if edits will be applied and another for actually applying
/// the edits.
///
/// See [`Mode`] to understand the behaviour of the modes.
#[derive(Debug)]
pub struct DryRunnable<'ast, T: TransformationRule<'ast>> {
    /// The wrapped [`TransformationRule`]
    rule: T,

    /// The current [`RunMode`]
    run_mode: RunMode,

    /// When `self.mode` is [`Mode::DryRun`] this will be `true` if edits *would* have happened.
    /// When `self.mode` is [`Mode::RealRun`] this will be `true` if edits *did* happen.
    did_edit: bool,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast, T: TransformationRule<'ast>> DryRunnable<'ast, T> {
    /// Creates a new [`DryRunTransformationRule`] with mode set to [`Mode::DryRun`].
    pub fn new(rule: T) -> Self {
        Self {
            rule,
            run_mode: RunMode::Dry,
            did_edit: false,
            _ast: PhantomData,
        }
    }

    /// Changes `self.run_mode` to [`RunMode::Real`]
    pub fn set_real_run_mode(&mut self) {
        self.run_mode = RunMode::Real;
    }

    /// When `self.mode` is [`Mode::DryRun`] this will return `true` if edits *would* have happened.
    /// When `self.mode` is [`Mode::RealRun`] this will  `true` if edits *did* happen.
    pub fn did_edit(&self) -> bool {
        self.did_edit
    }
}

/// The `run_mode` setting of a [`DryRunnable`].
#[derive(Debug, PartialEq)]
pub enum RunMode {
    /// Invoking [`TransformationRule::apply`] When this mode is set is equivalent to calling
    /// [`TransformationRule::would_edit`] on the wrapped rule.
    Dry,

    /// Invoking [`TransformationRule::apply`] When this mode is set is equivalent to calling
    /// [`TransformationRule::apply`] on the wrapped rule.
    Real,
}

impl<'ast, T: TransformationRule<'ast>> TransformationRule<'ast> for DryRunnable<'ast, T> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.run_mode == RunMode::Dry {
            self.did_edit = self.rule.would_edit(node_path, target_node) || self.did_edit;
        } else {
            self.did_edit = self.rule.apply(node_path, target_node)? || self.did_edit;
        }
        Ok(self.did_edit)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, target_node: &N) -> bool {
        self.did_edit = self.rule.would_edit(node_path, target_node) || self.did_edit;
        self.did_edit
    }
}

impl<'ast, T: TransformationRule<'ast>> Transform<'ast> for DryRunnable<'ast, T> {
    type Error = EqlMapperError;

    fn transform<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        mut target_node: N,
    ) -> Result<N, Self::Error> {
        self.apply(node_path, &mut target_node)?;
        Ok(target_node)
    }
}

#[impl_for_tuples(1, 16)]
impl<'ast> TransformationRule<'ast> for Tuple {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        let did_edit = false;
        for_tuples!( #(let did_edit = Tuple.apply(node_path, target_node)? || did_edit; )* );

        Ok(did_edit)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, target_node: &N) -> bool {
        let did_edit = false;
        for_tuples!( #(let did_edit = Tuple.would_edit(node_path, target_node) || did_edit; )* );

        did_edit
    }

    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        for_tuples!( #(Tuple.check_postcondition()?; )* );

        Ok(())
    }
}
