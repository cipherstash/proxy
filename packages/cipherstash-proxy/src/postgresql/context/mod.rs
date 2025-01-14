use super::{
    format_code::FormatCode,
    messages::{
        describe::{Describe, Target},
        Name,
    },
};
use crate::{log::CONTEXT, Identifier};
use cipherstash_config::{ColumnConfig, ColumnType};
use postgres_types::Type;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct Context {
    pub statements: Arc<RwLock<HashMap<Name, Arc<Statement>>>>,
    pub portals: Arc<RwLock<HashMap<Name, Arc<Portal>>>>,
    pub describe: Arc<RwLock<Option<Describe>>>,
    pub execute: Arc<RwLock<Name>>,
    pub schema_changed: Arc<RwLock<bool>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub identifier: Identifier,
    pub config: ColumnConfig,
    pub postgres_type: Type,
}

///
/// Type Analysed parameters and projection
///
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub param_columns: Vec<Option<Column>>,
    pub projection_columns: Vec<Option<Column>>,
    pub postgres_param_types: Vec<i32>,
}

///
/// Portal is a Statement with Bound variables
/// An Execute message will execute the statement with the variables associated during a Bind.
///
#[derive(Debug, Clone)]
pub struct Portal {
    // pub statement_name: Name,
    pub format_codes: Vec<FormatCode>,
    pub statement: Arc<Statement>,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: Arc::new(RwLock::new(HashMap::new())),
            portals: Arc::new(RwLock::new(HashMap::new())),
            describe: Arc::new(RwLock::from(None)),
            execute: Arc::new(RwLock::from(Name::unnamed())),
            schema_changed: Arc::new(RwLock::from(false)),
        }
    }

    pub fn describe(&mut self, describe: Describe) {
        debug!(target: CONTEXT, "Describe: {describe:?}");
        let _ = self
            .describe
            .write()
            .map(|mut guard| *guard = Some(describe));
    }

    pub fn execute(&mut self, name: Name) {
        debug!(target: CONTEXT, "Execute: {name:?}");
        let _ = self.execute.write().map(|mut guard| *guard = name);
    }

    pub fn add_statement(&mut self, name: Name, statement: Statement) {
        debug!(target: CONTEXT, "Statement: {name:?}");
        let _ = self
            .statements
            .write()
            .map(|mut guarded| guarded.insert(name, Arc::new(statement)));
    }

    pub fn add_portal(&mut self, name: Name, portal: Portal) {
        debug!(target: CONTEXT, "Portal: {name:?}");
        let _ = self
            .portals
            .write()
            .map(|mut guarded| guarded.insert(name, Arc::new(portal)));
    }

    pub fn get_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, "Get Statement: {name:?}");
        self.statements
            .read()
            .ok()
            .map(|guard| guard.get(name).cloned())?
    }

    pub fn get_statement_from_describe(&self) -> Option<Arc<Statement>> {
        self.describe.read().ok().map(|describe| {
            debug!(target: CONTEXT, "Describe: {describe:?}");
            match *describe {
                Some(Describe {
                    ref name,
                    target: Target::Portal,
                }) => self.get_portal_statement(name),
                Some(Describe {
                    ref name,
                    target: Target::PreparedStatement,
                }) => self.get_statement(name),
                _ => None,
            }
        })?
    }

    pub fn get_portal(&self, name: &Name) -> Option<Arc<Portal>> {
        debug!(target: CONTEXT, "Get Portal: {name:?}");
        self.portals
            .read()
            .ok()
            .map(|guard| guard.get(name).cloned())?
    }

    pub fn get_portal_statement(&self, name: &Name) -> Option<Arc<Statement>> {
        debug!(target: CONTEXT, "Get Portal: {name:?}");
        self.portals
            .read()
            .ok()
            .map(|guard| guard.get(name).map(|portal| portal.statement.clone()))?
    }

    pub fn get_portal_from_execute(&self) -> Option<Arc<Portal>> {
        self.execute.read().ok().map(|name| {
            debug!(target: CONTEXT, "Execute: {name:?}");
            self.get_portal(&name)
        })?
    }

    pub fn set_schema_changed(&self) {
        debug!(target: CONTEXT, "Schema changed");
        let _ = self.schema_changed.write().map(|mut guard| *guard = true);
    }

    pub fn schema_changed(&self) -> bool {
        self.schema_changed.read().ok().map_or(false, |s| *s)
    }
}

impl Column {
    pub fn new(identifier: Identifier, config: ColumnConfig) -> Column {
        let postgres_type = column_type_to_postgres_type(&config.cast_type);
        Column {
            identifier,
            config,
            postgres_type,
        }
    }
}

impl Statement {
    pub fn new(
        param_columns: Vec<Option<Column>>,
        projection_columns: Vec<Option<Column>>,
        postgres_param_types: Vec<i32>,
    ) -> Statement {
        Statement {
            param_columns,
            projection_columns,
            postgres_param_types,
        }
    }
}

impl Portal {
    pub fn new(statement: Arc<Statement>, format_codes: Vec<FormatCode>) -> Portal {
        Portal {
            format_codes,
            statement,
        }
    }

    // FormatCodes should not be None at this point
    // FormatCodes will be:
    //  - empty, in which case assume Text
    //  - single value, in which case use this for all columns
    //  - multiple values, in which case use the value for each column
    pub fn format_codes(&self, row_len: usize) -> Vec<FormatCode> {
        match self.format_codes.len() {
            0 => vec![FormatCode::Text; row_len],
            1 => {
                let format_code = match self.format_codes.first() {
                    Some(code) => *code,
                    None => FormatCode::Text,
                };
                vec![format_code; row_len]
            }
            _ => self.format_codes.clone(),
        }
    }
}

fn column_type_to_postgres_type(col_type: &ColumnType) -> postgres_types::Type {
    match col_type {
        ColumnType::Boolean => postgres_types::Type::BOOL,
        ColumnType::BigInt => postgres_types::Type::INT8,
        ColumnType::BigUInt => postgres_types::Type::INT8,
        ColumnType::Date => postgres_types::Type::DATE,
        ColumnType::Decimal => postgres_types::Type::NUMERIC,
        ColumnType::Float => postgres_types::Type::FLOAT8,
        ColumnType::Int => postgres_types::Type::INT4,
        ColumnType::SmallInt => postgres_types::Type::INT2,
        ColumnType::Timestamp => postgres_types::Type::TIMESTAMPTZ,
        ColumnType::Utf8Str => postgres_types::Type::TEXT,
        ColumnType::JsonB => postgres_types::Type::JSONB,
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Describe, Portal, Statement};
    use crate::{
        log,
        postgresql::messages::{describe::Target, Name},
    };

    fn statement() -> Statement {
        Statement {
            param_columns: vec![],
            projection_columns: vec![],
            postgres_param_types: vec![],
        }
    }

    #[test]
    pub fn test_get_statement_from_describe() {
        log::init();
        let mut context = Context::new();

        let name = Name("name".to_string());

        context.add_statement(name.clone(), statement());

        let statement = context.get_statement(&name).unwrap();

        let describe = Describe {
            name,
            target: Target::PreparedStatement,
        };
        context.describe(describe);

        let s = context.get_statement_from_describe().unwrap();

        assert_eq!(s, statement)
    }

    #[test]
    pub fn test_get_statement_from_execute() {
        log::init();

        let mut context = Context::new();

        let statement_name = Name("statement".to_string());
        let portal_name = Name("portal".to_string());

        // Add statement to context
        context.add_statement(statement_name.clone(), statement());

        // Get statement from context
        let statement = context.get_statement(&statement_name).unwrap();

        // Add portal pointing to statement to context
        let portal = Portal::new(statement.clone(), vec![]);
        context.add_portal(portal_name.clone(), portal);

        // Add statement name to execute context
        context.execute(portal_name);

        // Portal statement should be the right statement
        let portal = context.get_portal_from_execute().unwrap();
        assert_eq!(statement, portal.statement)
    }
}
