use std::{collections::HashMap, sync::Arc};

use sqltk::parser::ast::{self, Statement};
use sqltk::{AsNodeKey, NodeKey, Transformable};

use crate::unifier::EqlTerm;
use crate::{
    CastLiteralsAsEncrypted, CastParamsAsEncrypted, DryRunnable, EqlMapperError,
    FailOnPlaceholderChange, Param, PreserveEffectiveAliases, RewriteContainmentOps,
    RewriteStandardSqlFnsOnEqlTypes, TransformationRule,
};

use crate::unifier::{Projection, Type, Value};

/// A `TypeCheckedStatement` is returned from a successful call to [`crate::type_check`].
#[derive(Debug)]
pub struct TypeCheckedStatement<'ast> {
    /// A reference to the original unmodified [`Statement`].
    pub statement: &'ast Statement,

    /// The type of the resultset which will be generated when the statement is executed.
    pub projection: Projection,

    /// The types of all params discovered from [`Value::Placeholder`] nodes in the SQL statement.
    pub params: Vec<(Param, Value)>,

    /// The type ([`EqlTerm`]) and reference to an [`ast::Value`] nodes of all EQL literals from the SQL statement.
    pub literals: Vec<(EqlTerm, &'ast ast::Value)>,

    /// A [`HashMap`] of AST node (using [`NodeKey`] as the key) to [`Type`].  The map contains a `Type` for every node
    /// in the AST with the node type is one of: [`Statement`], [`Query`], [`Insert`], [`Delete`], [`Expr`],
    /// [`SetExpr`], [`Select`], [`SelectItem`], [`Vec<SelectItem>`], [`Function`], [`Values`], [`Value`].
    ///
    /// [`Query`]: sqltk::parser::ast::Query
    /// [`Insert`]: sqltk::parser::ast::Insert
    /// [`Delete`]: sqltk::parser::ast::Delete
    /// [`Expr`]: sqltk::parser::ast::Expr
    /// [`SetExpr`]: sqltk::parser::ast::SetExpr
    /// [`Select`]: sqltk::parser::ast::Select
    /// [`SelectItem`]: sqltk::parser::ast::SelectItem
    /// [`Function`]: sqltk::parser::ast::Function
    /// [`FunctionArgExpr`]: sqltk::parser::ast::FunctionArgExpr
    /// [`Values`]: sqltk::parser::ast::Values
    /// [`Value`]: sqltk::parser::ast::Value
    pub node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> TypeCheckedStatement<'ast> {
    pub(crate) fn new(
        statement: &'ast Statement,
        projection: Projection,
        params: Vec<(Param, Value)>,
        literals: Vec<(EqlTerm, &'ast ast::Value)>,
        node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
    ) -> Self {
        Self {
            statement,
            projection,
            params,
            literals,
            node_types,
        }
    }

    /// Returns `true` if one or more SQL param placeholders in the body has an EQL type, otherwise returns `false`.
    pub fn params_contain_eql(&self) -> bool {
        self.params.iter().any(|p| matches!(p.1, Value::Eql(_)))
    }

    /// Tests if a statement transformation is required. This works by executing all of the transformation rules but
    /// with AST modifications disabled.
    ///
    /// This method returns a `Result` instead of a plain `bool` because rule preconditions are checked and may
    /// fail.
    ///
    /// Returns `Ok(true)` if the AST would be modified, `Ok(false)` if the AST would not be modified.
    ///
    /// An `Err` indicates that a rule precondition failed.
    pub fn requires_transform(&self) -> bool {
        let mut dry_run_transformer = self.make_transformer(self.dummy_encrypted_literals());
        let _ = self.statement.apply_transform(&mut dry_run_transformer);

        dry_run_transformer.did_edit()
    }

    /// Transforms the SQL statement by replacing all plaintext literals with EQL equivalents
    /// and inserting EQL helper functions where necessary.
    pub fn transform(
        &self,
        encrypted_literals: HashMap<NodeKey<'ast>, sqltk::parser::ast::Value>,
    ) -> Result<Statement, EqlMapperError> {
        self.check_all_encrypted_literals_provided(&encrypted_literals)?;
        let mut transformer = self.make_transformer(encrypted_literals);
        transformer.set_real_run_mode();
        self.statement.apply_transform(&mut transformer)
    }

    pub fn literal_values(&self) -> &Vec<(EqlTerm, &'ast sqltk::parser::ast::Value)> {
        &self.literals
    }

    fn dummy_encrypted_literals(&self) -> HashMap<NodeKey<'ast>, ast::Value> {
        self.literals
            .iter()
            .map(|(_, v)| {
                (
                    NodeKey::new(*v),
                    ast::Value::SingleQuotedString("DUMMY".into()),
                )
            })
            .collect()
    }

    fn check_all_encrypted_literals_provided(
        &self,
        encrypted_literals: &HashMap<NodeKey<'ast>, sqltk::parser::ast::Value>,
    ) -> Result<(), EqlMapperError> {
        if self.count_not_null_literals() != encrypted_literals.len() {
            return Err(EqlMapperError::Transform(format!(
                "the number of encrypted literals is incorrect; expected {}, got {}",
                self.literals.len(),
                encrypted_literals.len(),
            )));
        }

        for (key, _) in encrypted_literals.iter() {
            if !self.literal_exists_for_node_key(*key) {
                return Err(EqlMapperError::Transform(String::from(
                    "encrypted literals refers to a literal node which is not present in the SQL statement"
                )));
            }
        }
        Ok(())
    }

    fn literal_exists_for_node_key(&self, key: NodeKey<'ast>) -> bool {
        self.literals
            .iter()
            .any(|(_, node)| node.as_node_key() == key)
    }

    fn count_not_null_literals(&self) -> usize {
        self.literals
            .iter()
            .filter(|(_, lit)| !matches!(lit, ast::Value::Null,))
            .count()
    }

    fn make_transformer(
        &self,
        encrypted_literals: HashMap<NodeKey<'ast>, sqltk::parser::ast::Value>,
    ) -> DryRunnable<'_, impl TransformationRule<'_>> {
        DryRunnable::new((
            RewriteStandardSqlFnsOnEqlTypes::new(Arc::clone(&self.node_types)),
            RewriteContainmentOps::new(Arc::clone(&self.node_types)),
            PreserveEffectiveAliases,
            CastLiteralsAsEncrypted::new(encrypted_literals),
            FailOnPlaceholderChange::new(),
            CastParamsAsEncrypted::new(Arc::clone(&self.node_types)),
        ))
    }
}
