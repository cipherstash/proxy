//! Types for representing and maintaining a lexical scope during AST traversal.
use sqlparser::ast::{Ident, ObjectName, Query, Statement};
use sqltk::{into_control_flow, Break, Visitable, Visitor};

use crate::inference::TypeError;
use crate::inference::{Constructor, Def, ProjectionColumn, Status, Type};
use crate::iterator_ext::IteratorExt;
use crate::model::SqlIdent;
use std::cell::RefCell;
use std::fmt::{self, Debug};
use std::rc::Rc;
use std::{cell::Cell, ops::ControlFlow};

use super::Relation;

use std::cell::OnceCell;

/// A lexical scope.
pub struct ScopeFrame {
    /// The items in scope.
    ///
    /// This is a `Vec` because the order of relations is important to be compatible with how databases deal with
    /// wildcard resolution.
    ///
    /// We can implement binary search or use a `BTreeMap` if/when it is deemed worthwhile.
    relations: Vec<Rc<Relation>>,

    /// This is computed the first time `resolve_wildcard` is called. The stored `Projection` never needs to be
    /// invalidated because by the time that any expression is resolving projections or identifiers from the scope, no
    /// more items will be published into the scope.
    ///
    /// The result is stored because otherwise `resolve_wildcard` would not be able to return a borrowed `Projection`.
    unqualified_wildcard: OnceCell<Rc<RefCell<Type>>>,

    /// Index of the parent scope.
    parent: Option<usize>,
}

impl Debug for ScopeFrame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let ScopeFrame {
            relations,
            parent,
            unqualified_wildcard,
            ..
        } = self;
        f.debug_struct("Scope")
            .field("relations", &relations.iter().collect::<Vec<_>>())
            .field("parent", &parent)
            .field("unqualified_wildcard", &unqualified_wildcard)
            .finish()
    }
}

impl ScopeFrame {
    pub(crate) fn new_root_scope() -> Self {
        Self {
            relations: Vec::new(),
            unqualified_wildcard: OnceCell::new(),
            parent: None,
        }
    }

    pub(crate) fn new_child_scope(&self, parent_idx: usize) -> Self {
        Self {
            relations: Vec::new(),
            unqualified_wildcard: OnceCell::new(),
            parent: Some(parent_idx),
        }
    }
}

#[derive(thiserror::Error, PartialEq, Eq, Debug, Clone)]
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
}

/// [`Visitor`] implementation that manages creation of lexical [`Scope`]s and the current active lexical scope.
pub struct Scope {
    /// Append-only backing store for lexical scopes.  [`Scope`] values keep track of their parent via indexing into
    /// this.
    scopes: Vec<ScopeFrame>,

    /// The stack of lexical scopes in an AST. The top of the stack represents the active lexical scope against with
    /// identifier and wildcard resolution will be performed.
    stack: Cell<Vec<usize>>,
}

/// Custom [`Debug`] implementation because [`FrozenVec`] does not implement `Debug` itself.
impl Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tmp = self.stack.take();
        let cloned = tmp.clone();
        self.stack.set(tmp);
        f.debug_struct("ScopeTracker")
            .field("scopes", &self.scopes.iter().collect::<Vec<_>>())
            .field("stack", &cloned)
            .finish()
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Scope {
    pub fn new() -> Self {
        Self {
            stack: Cell::new(vec![0]),
            scopes: vec![ScopeFrame::new_root_scope()],
        }
    }

    fn top_scope_mut(&mut self) -> Result<&mut ScopeFrame, ScopeError> {
        let stack = self.stack.take();
        match stack.last().cloned() {
            Some(idx) => {
                self.stack.set(stack);
                Ok(&mut self.scopes[idx])
            }
            None => Err(ScopeError::InvariantFailed(
                "Tried to access top scope of empty stack".to_string(),
            )),
        }
    }

    fn top_scope(&self) -> Result<&ScopeFrame, ScopeError> {
        let stack = self.stack.take();
        match stack.last().cloned() {
            Some(idx) => {
                self.stack.set(stack);
                Ok(&self.scopes[idx])
            }
            None => Err(ScopeError::InvariantFailed(
                "Tried to access top scope of empty stack".to_string(),
            )),
        }
    }

    fn push_scope(&mut self, scope: ScopeFrame) -> &ScopeFrame {
        self.scopes.push(scope);
        let mut stack = self.stack.take();
        let idx = stack.len();
        stack.push(idx);
        self.stack.set(stack);
        &self.scopes[idx]
    }

    fn push_new_root_scope(&mut self) -> &ScopeFrame {
        self.push_scope(ScopeFrame::new_root_scope())
    }

    fn push_new_child_scope(&mut self) -> Result<&ScopeFrame, ScopeError> {
        Ok(self.push_scope(self.top_scope()?.new_child_scope(self.scopes.len() - 1)))
    }

    fn pop_scope(&mut self) -> Result<(), ScopeError> {
        let mut stack = self.stack.take();
        if stack.is_empty() {
            return Err(ScopeError::InvariantFailed(
                "Tried to pop empty scope".to_string(),
            ));
        }
        stack.pop();
        self.stack.set(stack);
        Ok(())
    }

    fn lookup_scope(&self, scope_index: usize) -> Option<&ScopeFrame> {
        self.scopes.get(scope_index)
    }

    /// Resolves an unqualified wildcard. Resolution occurs in the current scope only  (i.e. does not look into parent
    /// scopes).
    pub fn resolve_wildcard(&self) -> Result<Rc<RefCell<Type>>, ScopeError> {
        let scope = self.top_scope()?;
        if scope.relations.is_empty() {
            Err(ScopeError::InvariantFailed(
                "Relations are empty".to_string(),
            ))
        } else {
            match scope.unqualified_wildcard.get() {
                Some(wildcard_ty) => Ok(wildcard_ty.clone()),
                None => {
                    let wildcard_ty: Vec<ProjectionColumn> = scope
                        .relations
                        .iter()
                        .map(|r| ProjectionColumn::new(r.projection_type.clone(), None))
                        .collect();

                    let resolved = wildcard_ty
                        .iter()
                        .map(|col| col.ty.borrow().status())
                        .fold(Status::Resolved, |acc, status| acc + status);

                    scope
                        .unqualified_wildcard
                        .set(Rc::new(RefCell::new(Type(
                            Def::Constructor(Constructor::Projection(Rc::new(RefCell::new(
                                wildcard_ty,
                            )))),
                            resolved,
                        ))))
                        .unwrap();

                    Ok(scope.unqualified_wildcard.get().unwrap().clone())
                }
            }
        }
    }

    /// Resolves a qualified wildcard. Resolution occurs in the current scope only (i.e. does not look into parent
    /// scopes).
    pub fn resolve_qualified_wildcard(
        &self,
        idents: &[Ident],
    ) -> Result<Rc<RefCell<Type>>, ScopeError> {
        let scope = self.top_scope()?;
        if idents.len() > 1 {
            return Err(ScopeError::UnsupportedCompoundIdentifierLength(
                idents
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join("."),
            ));
        }

        let sql_idents = idents.iter().map(SqlIdent::from).collect::<Vec<_>>();

        match scope
            .relations
            .iter()
            .find_unique(&|r| r.name.as_ref().map(SqlIdent::from).as_ref() == Some(&sql_idents[0]))
        {
            Ok(relation) => Ok(relation.projection_type.clone()),
            Err(_) => Err(ScopeError::NoMatch(idents[0].to_string())),
        }
    }

    fn try_match_projection(
        &self,
        ty: Rc<RefCell<Type>>,
    ) -> Result<Rc<RefCell<Vec<ProjectionColumn>>>, TypeError> {
        match &*ty.borrow() {
            Type(Def::Constructor(Constructor::Projection(columns)), _) => Ok(columns.clone()),
            other => Err(TypeError::Expected(format!(
                "expected projection but got: {other}"
            ))),
        }
    }

    /// Uniquely resolves an identifier against all relations that are in scope.
    pub fn resolve_ident(&self, ident: &Ident) -> Result<Rc<RefCell<Type>>, ScopeError> {
        let sql_ident = Some(SqlIdent::from(ident));
        let mut scope = self.top_scope()?;

        loop {
            let mut all_columns = scope
                .relations
                .iter()
                .map(|relation| self.try_match_projection(relation.projection_type.clone()))
                .try_fold(
                    Vec::<ProjectionColumn>::with_capacity(16),
                    |mut acc, columns| {
                        columns
                            .map(|columns| {
                                acc.extend(columns.borrow().iter().cloned());
                                acc
                            })
                            .map_err(|err| ScopeError::TypeError(Box::new(err)))
                    },
                )?
                .into_iter();

            match all_columns
                .try_find_unique(&|col| col.alias.as_ref().map(SqlIdent::from) == sql_ident)
            {
                Ok(Some(col)) => return Ok(col.ty.clone()),
                Err(_) => return Err(ScopeError::AmbiguousMatch(ident.to_string())),
                Ok(None) => {
                    if let Some(parent_index) = scope.parent {
                        match self.lookup_scope(parent_index) {
                            Some(parent) => {
                                scope = parent;
                                continue;
                            }
                            None => {
                                return Err(ScopeError::InvariantFailed(
                                    "Failed to resolve parent scope".to_string(),
                                ))
                            }
                        }
                    } else {
                        return Err(ScopeError::NoMatch(ident.to_string()));
                    }
                }
            }
        }
    }

    /// Resolves usage of a compound identifier.
    ///
    /// Note that currently only compound identifier of length 2 are supported
    /// and resolution will fail if the identifier has more than two parts.
    pub fn resolve_compound_ident(
        &self,
        idents: &[Ident],
    ) -> Result<Rc<RefCell<Type>>, ScopeError> {
        if idents.len() != 2 {
            return Err(ScopeError::InvariantFailed(
                "Unsupported compound identifier length (max = 2)".to_string(),
            ));
        }

        let first_ident = SqlIdent::from(&idents[0]);
        let second_ident = SqlIdent::from(&idents[1]);
        let mut scope = self.top_scope()?;

        loop {
            let mut relations = scope.relations.iter();

            match relations.try_find_unique(&|relation| {
                relation.name.as_ref().map(SqlIdent::from).as_ref() == Some(&first_ident)
            }) {
                Ok(Some(named_relation)) => {
                    let columns = self
                        .try_match_projection(named_relation.projection_type.clone())
                        .map_err(|err| ScopeError::TypeError(Box::new(err)))?;
                    let columns = columns.borrow();
                    let mut columns = columns.iter();

                    match columns.try_find_unique(&|column| {
                        column.alias.as_ref().map(SqlIdent::from).as_ref() == Some(&second_ident)
                    }) {
                        Ok(Some(projection_column)) => {
                            return Ok(projection_column.ty.clone());
                        }
                        Ok(None) => {
                            return Err(ScopeError::NoMatch(format!(
                                "{}.{}",
                                first_ident, second_ident
                            )))
                        }
                        Err(_) => {
                            return Err(ScopeError::AmbiguousMatch(format!(
                                "{}.{}",
                                first_ident, second_ident
                            )))
                        }
                    }
                }
                Ok(None) => {
                    if let Some(parent_index) = scope.parent {
                        match self.lookup_scope(parent_index) {
                            Some(parent) => {
                                scope = parent;
                                continue;
                            }
                            None => {
                                return Err(ScopeError::InvariantFailed(
                                    "Failed to resolve parent scope".to_string(),
                                ))
                            }
                        }
                    } else {
                        return Err(ScopeError::NoMatch(format!(
                            "{}.{}",
                            first_ident, second_ident
                        )));
                    }
                }
                Err(_) => {
                    return Err(ScopeError::NoMatch(format!(
                        "{}.{}",
                        first_ident, second_ident
                    )))
                }
            }
        }
    }

    /// Add a table/view/subquery to the current scope.
    pub fn add_relation(&mut self, relation: Relation) -> Result<Rc<Relation>, ScopeError> {
        let current_scope = self.top_scope_mut()?;
        current_scope.relations.push(Rc::new(relation));
        Ok(current_scope.relations[current_scope.relations.len() - 1].clone())
    }

    pub fn resolve_relation(&self, name: &ObjectName) -> Result<&Relation, ScopeError> {
        if name.0.len() > 1 {
            return Err(ScopeError::UnsupportedSqlFeature(
                "Tried to resolve a relation using a compound identifier".into(),
            ));
        }

        let mut current_scope = self.top_scope()?;
        let ident = &SqlIdent::from(name.0.last().unwrap());

        loop {
            match current_scope.relations.iter().try_find_unique(&|relation| {
                relation.name.as_ref().map(SqlIdent::from).as_ref() == Some(ident)
            }) {
                Ok(Some(found)) => return Ok(found),
                Ok(None) => match current_scope.parent {
                    Some(parent_index) => match self.lookup_scope(parent_index) {
                        Some(parent) => {
                            current_scope = parent;
                            continue;
                        }
                        None => {
                            return Err(ScopeError::InvariantFailed(
                                "Failed to resolve parent scope".to_string(),
                            ))
                        }
                    },
                    None => return Err(ScopeError::NoMatch(ident.to_string())),
                },
                Err(_) => return Err(ScopeError::NoMatch(ident.to_string())),
            }
        }
    }
}

impl<'ast> Visitor<'ast> for Scope {
    type Error = ScopeError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if node.is::<Statement>() {
            self.push_new_root_scope();
        }

        if node.is::<Query>() {
            into_control_flow(self.push_new_child_scope())?;
        }

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if node.is::<Statement>() {
            into_control_flow(self.pop_scope())?;
        }

        if node.is::<Query>() {
            into_control_flow(self.pop_scope())?;
        }

        ControlFlow::Continue(())
    }
}
