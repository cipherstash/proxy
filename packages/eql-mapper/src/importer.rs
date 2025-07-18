use crate::{
    inference::{unifier::Type, TypeError, TypeRegistry},
    model::{SchemaError, TableResolver},
    unifier::{Projection, Value},
    Relation, ScopeError, ScopeTracker,
};
use sqltk::parser::ast::{
    Cte, Ident, Insert, ObjectNamePart, TableAlias, TableFactor, TableObject,
};
use sqltk::{Break, Visitable, Visitor};
use std::{cell::RefCell, fmt::Debug, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc};

/// `Importer` is a [`Visitor`] implementation that brings projections (from "FROM" clauses and subqueries) into lexical scope.
// TODO: If Importer was refactored to be a suite of helper functions then the inferencer coud simply ask it to provide Types
// and we'd remove the "everything is coupled to the TypeRegistry thing"
pub struct Importer<'ast> {
    table_resolver: Arc<TableResolver>,
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> Importer<'ast> {
    pub fn new(
        table_resolver: impl Into<Arc<TableResolver>>,
        registry: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
    ) -> Self {
        Self {
            registry: registry.into(),
            table_resolver: table_resolver.into(),
            scope_tracker: scope.into(),
            _ast: PhantomData,
        }
    }

    fn update_scope_for_insert_statement(&mut self, insert: &Insert) -> Result<(), ImportError> {
        if let Insert {
            table: TableObject::TableName(table_name),
            table_alias,
            ..
        } = insert
        {
            let table = self.table_resolver.resolve_table(table_name)?;

            let projection = Projection::new_from_schema_table(table.clone());

            self.scope_tracker.borrow_mut().add_relation(Relation {
                name: table_alias.clone(),
                projection_type: Type::Value(Value::Projection(projection)).into(),
            })?;

            Ok(())
        } else {
            Err(ImportError::Unsupported(
                "unsupported TableObject variant in Insert".to_string(),
            ))
        }
    }

    fn update_scope_for_cte(&mut self, cte: &'ast Cte) -> Result<(), ImportError> {
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

        let mut registry = self.registry.borrow_mut();
        let query_ty = registry.get_node_type(query);

        self.scope_tracker.borrow_mut().add_relation(Relation {
            name: Some(alias.clone()),
            projection_type: query_ty,
        })?;

        Ok(())
    }

    fn update_scope_for_table_factor(
        &mut self,
        table_factor: &'ast TableFactor,
    ) -> Result<(), ImportError> {
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
                    None => {
                        let ObjectNamePart::Identifier(ident) = name.0.last().unwrap();
                        Ok(ident)
                    }
                };

                let mut scope_tracker = self.scope_tracker.borrow_mut();

                if scope_tracker.resolve_relation(name).is_err() {
                    let table = self.table_resolver.resolve_table(name)?;

                    let projection = Projection::new_from_schema_table(table.clone());

                    scope_tracker.add_relation(Relation {
                        name: record_as.cloned().ok(),
                        projection_type: Type::Value(Value::Projection(projection)).into(),
                    })?;
                }
            }

            TableFactor::Table { with_hints, .. } if !with_hints.is_empty() => {
                return Err(ImportError::Unsupported(
                    "Table: MySQL 'hints' unsupported".to_owned(),
                ))
            }

            TableFactor::Table { partitions, .. } if !partitions.is_empty() => {
                return Err(ImportError::Unsupported(
                    "Table: MySQL partition selection unsupported".to_owned(),
                ))
            }

            TableFactor::Table { args: Some(_), .. } => {
                return Err(ImportError::Unsupported(
                    "Table: table-valued function".to_owned(),
                ))
            }

            TableFactor::Table {
                version: Some(_), ..
            } => {
                return Err(ImportError::Unsupported(
                    "Table: version qualifier".to_owned(),
                ))
            }

            TableFactor::Derived {
                lateral: _,
                subquery,
                alias,
            } => {
                let projection_type = self.registry.borrow_mut().get_node_type(&*subquery.body);

                self.scope_tracker.borrow_mut().add_relation(Relation {
                    name: alias.clone().map(|a| a.name.clone()),
                    projection_type,
                })?;
            }

            #[allow(unused_variables)]
            TableFactor::TableFunction { expr, alias } => {
                return Err(ImportError::Unsupported("TableFunction".to_owned()))
            }

            #[allow(unused_variables)]
            TableFactor::Function {
                lateral,
                name,
                args,
                alias,
            } => return Err(ImportError::Unsupported("Function".to_owned())),

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
                return Err(ImportError::Unsupported("UNNEST".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::XmlTable {
                namespaces,
                row_expression,
                passing,
                columns,
                alias,
            } => {
                return Err(ImportError::Unsupported("XML table".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::JsonTable {
                json_expr,
                json_path,
                columns,
                alias,
            } => {
                return Err(ImportError::Unsupported("JSON table".to_owned()));
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
                return Err(ImportError::Unsupported("MATCH RECOGNIZE".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::OpenJsonTable {
                json_expr,
                json_path,
                columns,
                alias,
            } => {
                return Err(ImportError::Unsupported("OPEN JSON TABLE".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::NestedJoin {
                table_with_joins,
                alias,
            } => {
                return Err(ImportError::Unsupported("NESTED JOIN".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::Pivot {
                table,
                aggregate_functions,
                value_column,
                value_source,
                default_on_null,
                alias,
            } => {
                return Err(ImportError::Unsupported("PIVOT".to_owned()));
            }

            #[allow(unused_variables)]
            TableFactor::Unpivot {
                table,
                value,
                name,
                columns,
                alias,
            } => {
                return Err(ImportError::Unsupported("UNPIVOT".to_owned()));
            }
        }

        Ok(())
    }

    fn validate_table_alias(alias: &TableAlias) -> Result<&Ident, ImportError> {
        match alias {
            TableAlias { name, columns } if columns.is_empty() => Ok(name),
            _ => Err(ImportError::Unsupported(alias.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("Invariant failed: no columns in CTE: {}", _0)]
    NoColumnsInCte(String),

    #[error("Unsupported table source: {}", _0)]
    Unsupported(String),

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
        // [`sqltk::parser::ast::ObjectName`], and `ObjectName` is used in contexts where it *should not* bring anything in
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
            if let Err(err) = self.update_scope_for_cte(cte) {
                return ControlFlow::Break(Break::Err(err));
            }
        };

        if let Some(table_factor) = node.downcast_ref::<TableFactor>() {
            if let Err(err) = self.update_scope_for_table_factor(table_factor) {
                return ControlFlow::Break(Break::Err(err));
            }
        };

        ControlFlow::Continue(())
    }
}
