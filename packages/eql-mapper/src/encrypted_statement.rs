use std::{collections::HashMap, sync::Arc};

use sqlparser::ast::Expr;
use sqltk::{NodeKey, NodePath, Transform, Visitable};

use crate::{
    EqlMapperError, FailOnPlaceholderChange, GroupByEqlCol, PreserveEffectiveAliases,
    ReplacePlaintextEqlLiterals, TransformationRule, Type, UseEquivalentSqlFuncForEqlTypes,
    WrapEqlColsInOrderByWithOreFn, WrapGroupedEqlColInAggregateFn,
};

#[derive(Debug)]
pub(crate) struct EncryptedStatement<'ast> {
    transformation_rules: (
        WrapGroupedEqlColInAggregateFn<'ast>,
        GroupByEqlCol<'ast>,
        WrapEqlColsInOrderByWithOreFn<'ast>,
        PreserveEffectiveAliases,
        ReplacePlaintextEqlLiterals<'ast>,
        UseEquivalentSqlFuncForEqlTypes<'ast>,
        FailOnPlaceholderChange,
    ),
}

impl<'ast> EncryptedStatement<'ast> {
    pub(crate) fn new(
        encrypted_literals: HashMap<NodeKey<'ast>, Expr>,
        node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
    ) -> Self {
        Self {
            transformation_rules: (
                WrapGroupedEqlColInAggregateFn::new(Arc::clone(&node_types)),
                GroupByEqlCol::new(Arc::clone(&node_types)),
                WrapEqlColsInOrderByWithOreFn::new(Arc::clone(&node_types)),
                PreserveEffectiveAliases,
                ReplacePlaintextEqlLiterals::new(encrypted_literals),
                UseEquivalentSqlFuncForEqlTypes::new(Arc::clone(&node_types)),
                FailOnPlaceholderChange,
            ),
        }
    }
}

/// Applies all of the transormation rules from the `EncryptedStatement`.
impl<'ast> Transform<'ast> for EncryptedStatement<'ast> {
    type Error = EqlMapperError;

    fn transform<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        mut target_node: N,
    ) -> Result<N, Self::Error> {
        self.transformation_rules
            .apply(node_path, &mut target_node)?;

        Ok(target_node)
    }

    fn check_postcondition(&self) -> Result<(), Self::Error> {
        self.transformation_rules.check_postcondition()
    }
}
