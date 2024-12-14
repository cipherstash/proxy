use std::{cell::RefCell, fmt::Debug, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc};

use sqlparser::ast::{Cte, Ident, Insert, TableAlias, TableFactor};
use sqltk::{Break, Visitable, Visitor};
use tracing::info;

use crate::{
    inference::{unifier::{Constructor, Def, ProjectionColumn, Status, Type}, TypeError, TypeRegistry},
    model::{Relation, Schema, SchemaError},
    ScopeError, ScopeTracker,
};

/// `Importer` is a [`Visitor`] implementation that brings projections (from "FROM" clauses and subqueries) into lexical scope.
#[derive(Debug)]
pub struct Importer<'ast> {
    schema: Arc<Schema>,
    reg: Rc<RefCell<TypeRegistry<'ast>>>,
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> Importer<'ast> {
    pub fn new(
        schema: impl Into<Arc<Schema>>,
        reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
    ) -> Self {
        Self {
            reg: reg.into(),
            schema: schema.into(),
            scope_tracker: scope.into(),
            _ast: PhantomData,
        }
    }

    fn update_scope_for_insert_statement(&mut self, insert: &Insert) -> Result<(), ImportError> {
        let Insert {
            table_name,
            table_alias,
            ..
        } = insert;

        let table = self.schema.resolve_table(table_name.0.last().unwrap())?;

        self.scope_tracker.borrow_mut().add_relation(Relation {
            name: table_alias.clone(),
            projection_type: Type(
                Def::Constructor(Constructor::Projection(Rc::new(RefCell::new(Vec::<
                    ProjectionColumn,
                >::from(
                    &*table
                ))))),
                Status::Resolved,
            )
            .wrap(),
        })?;

        Ok(())
    }

    fn update_scope_for_cte(&mut self, cte: &'ast Cte) -> Result<(), ImportError> {
        info!("update_scope_for_cte");

        let Cte {
            alias: TableAlias {
                name: alias,
                columns,
            },
            query,
            ..
        } = cte;

        if !columns.is_empty() {
            return Err(ImportError::NoColumnsInCte(cte.to_string()));
        }

        let mut reg = self.reg.borrow_mut();
        let projection_type = reg.get_type(&**query);

        self.scope_tracker.borrow_mut().add_relation(Relation {
            name: Some(alias.clone()),
            projection_type,
        })?;

        Ok(())
    }

    fn update_scope_for_table_factor(
        &mut self,
        table_factor: &'ast TableFactor,
    ) -> Result<(), ImportError> {
        info!("update_scope_for_table_factor");

        match table_factor {
            TableFactor::Table {
                name,
                alias,
                args: None,
                version: None,
                ..
            } => {
                let record_as = match alias {
                    Some(alias) => Self::validate_table_alias(alias),
                    None => Ok(name.0.last().unwrap()),
                };

                let mut scope_tracker = self.scope_tracker.borrow_mut();

                if scope_tracker.resolve_relation(name).is_err() {
                    let table = self.schema.resolve_table(name.0.last().unwrap())?;

                    scope_tracker.add_relation(Relation {
                        name: record_as.cloned().ok(),
                        projection_type: Type(
                            Def::Constructor(Constructor::Projection(Rc::new(RefCell::new(
                                Vec::<ProjectionColumn>::from(&*table),
                            )))),
                            Status::Resolved,
                        )
                        .wrap(),
                    })?;
                }
            }

            TableFactor::Table { with_hints, .. } if !with_hints.is_empty() => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "Table: MySQL 'hints' unsupported".to_owned(),
                ))
            }

            TableFactor::Table { partitions, .. } if !partitions.is_empty() => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "Table: MySQL partition selection unsupported".to_owned(),
                ))
            }

            TableFactor::Table { args: Some(_), .. } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "Table: table-valued function".to_owned(),
                ))
            }

            TableFactor::Table {
                version: Some(_), ..
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "Table: version qualifier".to_owned(),
                ))
            }

            TableFactor::Derived {
                lateral: _,
                subquery,
                alias,
            } => {
                let projection_type = self.reg.borrow_mut().get_type(&*subquery.body);

                self.scope_tracker.borrow_mut().add_relation(Relation {
                    name: alias.clone().map(|a| a.name.clone()),
                    projection_type,
                })?;
            }

            #[allow(unused_variables)]
            TableFactor::TableFunction { expr, alias } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "TableFunction".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::Function {
                lateral,
                name,
                args,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "Function".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::UNNEST {
                alias,
                array_exprs,
                with_offset,
                with_offset_alias,
                with_ordinality,
            } => {
                // all exprs in array_exprs must have same type
                // if with_offset is true
                //   generate a two column projection
                // else
                //   generate a single column projection
                // end
                //
                // if alias is Some(_) then that is the projection name
                // if alias is None then the projection is anonymous, like a wildcard.
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "UNNEST".to_owned(),
                ));
            }

            #[allow(unused_variables)]
            TableFactor::JsonTable {
                json_expr,
                json_path,
                columns,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "JSON_TABLE".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::NestedJoin {
                table_with_joins,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "NestedJoin".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::Pivot {
                table,
                aggregate_functions,
                value_source,
                default_on_null,
                value_column,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "PIVOT".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::Unpivot {
                table,
                value,
                name,
                columns,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "UNPIVOT".to_owned(),
                ))
            }

            #[allow(unused_variables)]
            TableFactor::MatchRecognize {
                table,
                partition_by,
                order_by,
                measures,
                rows_per_match,
                after_match_skip,
                pattern,
                symbols,
                alias,
            } => {
                return Err(ImportError::UnsupportedTableFactorVariant(
                    "MATCH_RECOGNIZE".to_owned(),
                ))
            }
        }

        Ok(())
    }

    fn validate_table_alias(alias: &TableAlias) -> Result<&Ident, ImportError> {
        match alias {
            TableAlias { name, columns } if columns.is_empty() => Ok(name),
            _ => Err(ImportError::UnsupportTableAliasVariant(alias.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("Invariant failed: no columns in CTE: {}", _0)]
    NoColumnsInCte(String),

    #[error("Unsupported TableFactor variant: {}", _0)]
    UnsupportedTableFactorVariant(String),

    #[error("Unsupported TableAlias variant: {}", _0)]
    UnsupportTableAliasVariant(String),

    #[error(transparent)]
    SchemaError(#[from] SchemaError),

    #[error(transparent)]
    ScopeError(#[from] ScopeError),

    #[error("Expected projection")]
    ExpectedProjection,

    #[error(transparent)]
    TypeError(#[from] TypeError),
}

impl<'ast> Visitor<'ast> for Importer<'ast> {
    type Error = ImportError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        // Most nodes that bring relations into scope use `exit` but in Insert's case we need to use `enter` because
        //
        // 1. There is no approprate child AST node of [`Insert`] on which to listen for an `exit` from except
        // [`sqlparser::ast::ObjectName`], and `ObjectName` is used in contexts where it *should not* bring anything in
        // to scope (it is not only used to identify tables).
        //
        // 2. Child nodes of the `Insert` need to resolve identifiers in the context of the scope, so exit would be too
        // late.
        if let Some(insert) = node.downcast_ref::<Insert>() {
            if let Err(err) = self.update_scope_for_insert_statement(insert) {
                return ControlFlow::Break(Break::Err(err));
            }
        }

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(cte) = node.downcast_ref::<Cte>() {
            info!("CTE {}", cte);
            if let Err(err) = self.update_scope_for_cte(cte) {
                return ControlFlow::Break(Break::Err(err));
            }
        };

        if let Some(table_factor) = node.downcast_ref::<TableFactor>() {
            info!("TableFactor {}", table_factor);
            if let Err(err) = self.update_scope_for_table_factor(table_factor) {
                return ControlFlow::Break(Break::Err(err));
            }
        };

        ControlFlow::Continue(())
    }
}
