//! Types for representing and maintaining a lexical scope during AST traversal.
use crate::inference::unifier::{ProjectionColumn, Type};
use crate::inference::TypeError;
use crate::iterator_ext::IteratorExt;
use crate::model::SqlIdent;
use crate::unifier::{Projection, Value};
use crate::Relation;
use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart, Query, Statement};
use sqltk::{into_control_flow, Break, Visitable, Visitor};
use std::cell::RefCell;
use std::fmt::Debug;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::sync::Arc;

/// [`Visitor`] implementation that manages creation of lexical [`Scope`]s and the current active lexical scope.
#[derive(Debug)]
pub struct ScopeTracker<'ast> {
    stack: Vec<Rc<RefCell<Scope<'ast>>>>,
}

impl<'ast> ScopeTracker<'ast> {
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(64),
        }
    }

    fn current_scope(&self) -> Result<Rc<RefCell<Scope<'ast>>>, ScopeError> {
        self.stack.last().cloned().ok_or(ScopeError::NoCurrentScope)
    }

    /// Resolves an unqualified wildcard. Resolution occurs in the current scope only  (i.e. does not look into parent
    /// scopes).
    pub(crate) fn resolve_wildcard(&self) -> Result<Arc<Type>, ScopeError> {
        self.current_scope()?.borrow().resolve_wildcard()
    }

    /// Resolves a qualified wildcard. Resolution occurs in the current scope only (i.e. does not look into parent
    /// scopes).
    pub(crate) fn resolve_qualified_wildcard(
        &self,
        idents: &ObjectName,
    ) -> Result<Arc<Type>, ScopeError> {
        self.current_scope()?
            .borrow()
            .resolve_qualified_wildcard(idents)
    }

    /// Uniquely resolves an identifier against all relations that are in scope.
    pub(crate) fn resolve_ident(&self, ident: &Ident) -> Result<Arc<Type>, ScopeError> {
        self.current_scope()?.borrow().resolve_ident(ident)
    }

    /// Resolves usage of a compound identifier.
    ///
    /// Note that currently only compound identifier of length 2 are supported
    /// and resolution will fail if the identifier has more than two parts.
    pub(crate) fn resolve_compound_ident(&self, idents: &[Ident]) -> Result<Arc<Type>, ScopeError> {
        self.current_scope()?
            .borrow()
            .resolve_compound_ident(idents)
    }

    /// Add a table/view/subquery to the current scope.
    pub(crate) fn add_relation(&mut self, relation: Relation) -> Result<(), ScopeError> {
        self.current_scope()?.borrow_mut().add_relation(relation)
    }

    pub(crate) fn resolve_relation(&self, name: &ObjectName) -> Result<Rc<Relation>, ScopeError> {
        self.current_scope()?.borrow().resolve_relation(name)
    }
}

/// A lexical scope.
#[derive(Debug)]
pub(crate) struct Scope<'ast> {
    /// The items in scope.
    ///
    /// This is a `Vec` because the order of relations is important to be compatible with how databases deal with
    /// wildcard resolution.
    ///
    /// We can implement binary search or use a `BTreeMap` if/when it is deemed worthwhile.
    relations: Vec<Rc<Relation>>,

    /// The parent scope.
    parent: Option<Rc<RefCell<Scope<'ast>>>>,
}

impl<'ast> Scope<'ast> {
    fn new_root() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            relations: Vec::new(),
            parent: None,
        }))
    }

    fn new_child(parent: &Rc<RefCell<Scope<'ast>>>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            relations: Vec::new(),
            parent: Some(parent.clone()),
        }))
    }

    pub(crate) fn resolve_wildcard(&self) -> Result<Arc<Type>, ScopeError> {
        if self.relations.is_empty() {
            match &self.parent {
                Some(parent) => parent.borrow().resolve_wildcard(),
                None => Err(ScopeError::NoMatch(String::from("empty scope"))),
            }
        } else {
            let columns: Vec<ProjectionColumn> = self
                .relations
                .iter()
                .map(|r| ProjectionColumn::new(r.projection_type.clone(), None))
                .collect();

            Ok(Type::Value(Value::Projection(Projection::new(columns))).into())
        }
    }

    pub(crate) fn resolve_qualified_wildcard(
        &self,
        name: &ObjectName,
    ) -> Result<Arc<Type>, ScopeError> {
        if name.0.len() > 1 {
            return Err(ScopeError::UnsupportedCompoundIdentifierLength(
                name.to_string(),
            ));
        }

        if self.relations.is_empty() {
            match &self.parent {
                Some(parent) => parent.borrow().resolve_qualified_wildcard(name),
                None => Err(ScopeError::NoMatch(String::from("empty scope"))),
            }
        } else {
            let sql_idents = name
                .0
                .iter()
                .map(|ObjectNamePart::Identifier(ident)| ident)
                .map(SqlIdent::from)
                .collect::<Vec<_>>();

            match self.relations.iter().find_unique(&|r| {
                r.name.as_ref().map(SqlIdent::from).as_ref() == Some(&sql_idents[0])
            }) {
                Ok(relation) => Ok(relation.projection_type.clone()),
                Err(_) => Err(ScopeError::NoMatch(sql_idents[0].to_string())),
            }
        }
    }

    pub(crate) fn resolve_ident(&self, ident: &Ident) -> Result<Arc<Type>, ScopeError> {
        if self.relations.is_empty() {
            match &self.parent {
                Some(parent) => parent.borrow().resolve_ident(ident),
                None => Err(ScopeError::NoMatch(String::from("empty scope"))),
            }
        } else {
            let sql_ident = Some(SqlIdent::from(ident));

            let mut all_columns = self
                .relations
                .iter()
                .map(|relation| self.try_match_projection(relation.projection_type.clone()))
                .try_fold(
                    Vec::<ProjectionColumn>::with_capacity(16),
                    |mut acc, columns| {
                        columns
                            .map(|columns| {
                                acc.extend(columns.iter().cloned());
                                acc
                            })
                            .map_err(|err| ScopeError::TypeError(Box::new(err)))
                    },
                )?
                .into_iter();

            match all_columns
                .try_find_unique(&|col| col.alias.as_ref().map(SqlIdent::from) == sql_ident)
            {
                Ok(Some(col)) => Ok(col.ty),
                Err(_) => Err(ScopeError::AmbiguousMatch(ident.to_string())),
                Ok(None) => match &self.parent {
                    Some(parent) => parent.borrow().resolve_ident(ident),
                    None => Err(ScopeError::NoMatch(format!(
                        "identifier {} not found in scope",
                        ident
                    ))),
                },
            }
        }
    }

    pub(crate) fn resolve_compound_ident(&self, idents: &[Ident]) -> Result<Arc<Type>, ScopeError> {
        if idents.len() != 2 {
            return Err(ScopeError::InvariantFailed(
                "Unsupported compound identifier length (max = 2)".to_string(),
            ));
        }

        let first_ident = SqlIdent::from(&idents[0]);
        let second_ident = SqlIdent::from(&idents[1]);

        let mut relations = self.relations.iter();

        match relations.try_find_unique(&|relation| {
            relation.name.as_ref().map(SqlIdent::from).as_ref() == Some(&first_ident)
        }) {
            Ok(Some(named_relation)) => {
                let columns = self
                    .try_match_projection(named_relation.projection_type.clone())
                    .map_err(|err| ScopeError::TypeError(Box::new(err)))?;
                let mut columns = columns.iter();

                match columns.try_find_unique(&|column| {
                    column.alias.as_ref().map(SqlIdent::from).as_ref() == Some(&second_ident)
                }) {
                    Ok(Some(projection_column)) => Ok(projection_column.ty.clone()),
                    Ok(None) | Err(_) => Err(ScopeError::NoMatch(format!(
                        "{}.{}",
                        first_ident, second_ident
                    ))),
                }
            }
            Ok(None) | Err(_) => match &self.parent {
                Some(parent) => parent.borrow().resolve_compound_ident(idents),
                None => Err(ScopeError::NoMatch(format!(
                    "{}.{}",
                    first_ident, second_ident
                ))),
            },
        }
    }

    fn add_relation(&mut self, relation: Relation) -> Result<(), ScopeError> {
        self.relations.push(Rc::new(relation));
        Ok(())
    }

    pub(crate) fn resolve_relation(&self, name: &ObjectName) -> Result<Rc<Relation>, ScopeError> {
        if name.0.len() > 1 {
            return Err(ScopeError::UnsupportedSqlFeature(
                "Tried to resolve a relation using a compound identifier".into(),
            ));
        }

        let ObjectNamePart::Identifier(ident) = name.0.last().unwrap();
        let ident = SqlIdent(ident);

        match self.relations.iter().try_find_unique(&|relation| {
            relation.name.as_ref().map(SqlIdent::from).as_ref() == Some(&ident)
        }) {
            Ok(Some(found)) => Ok(found.clone()),
            Ok(None) => match &self.parent {
                Some(parent) => Ok(parent.borrow().resolve_relation(name)?),
                None => Err(ScopeError::NoMatch(ident.to_string())),
            },
            Err(_) => Err(ScopeError::NoMatch(ident.to_string())),
        }
    }

    fn try_match_projection(&self, ty: Arc<Type>) -> Result<Vec<ProjectionColumn>, TypeError> {
        match &*ty {
            Type::Value(Value::Projection(projection)) => {
                Ok(Vec::from_iter(projection.columns().iter().cloned()))
            }
            other => Err(TypeError::Expected(format!(
                "expected projection but got: {other}"
            ))),
        }
    }
}

#[derive(thiserror::Error, PartialEq, Eq, Debug)]
pub enum ScopeError {
    #[error("No match: no matches for identifier '{}'", _0)]
    NoMatch(String),

    #[error("Ambiguous: multiple matches for identifier '{}'", _0)]
    AmbiguousMatch(String),

    #[error("Unsupported compound identifier length for ident '{}'", _0)]
    UnsupportedCompoundIdentifierLength(String),

    #[error("Invariant failed: {}", _0)]
    InvariantFailed(String),

    #[error("Unsupported SQL feature: {}", _0)]
    UnsupportedSqlFeature(String),

    #[error(transparent)]
    TypeError(Box<TypeError>),

    #[error("No current scope")]
    NoCurrentScope,
}

impl<'ast> Visitor<'ast> for ScopeTracker<'ast> {
    type Error = ScopeError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if node.downcast_ref::<Statement>().is_some() {
            let root = Scope::new_root();
            self.stack.push(root.clone());
            return ControlFlow::Continue(());
        }

        if node.downcast_ref::<Query>().is_some() {
            match self.stack.last() {
                Some(scope) => {
                    let child = Scope::new_child(scope);
                    self.stack.push(child.clone());
                    return ControlFlow::Continue(());
                }
                None => return ControlFlow::Break(Break::Err(ScopeError::NoCurrentScope)),
            }
        }

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if node.downcast_ref::<Statement>().is_some() {
            return into_control_flow(self.stack.pop().ok_or(ScopeError::NoCurrentScope));
        }

        if node.downcast_ref::<Query>().is_some() {
            return into_control_flow(self.stack.pop().ok_or(ScopeError::NoCurrentScope));
        }

        ControlFlow::Continue(())
    }
}
