use crate::error::Error;
use crate::prometheus::STATEMENTS_TOTAL;
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::parser::Parser;

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct SqlParser;

impl SqlParser {
    /// Parse a SQL statement string into an SqlParser AST
    pub fn parse_statement(statement: &str) -> Result<ast::Statement, Error> {
        let statement = Parser::new(&DIALECT)
            .try_with_sql(statement)?
            .parse_statement()?;

        counter!(STATEMENTS_TOTAL).increment(1);

        Ok(statement)
    }

    /// Parse a SQL String potentially containing multiple statements into parsed SqlParser AST
    pub fn parse_statements(statement: &str) -> Result<Vec<ast::Statement>, Error> {
        let statement = Parser::new(&DIALECT)
            .try_with_sql(statement)?
            .parse_statements()?;

        counter!(STATEMENTS_TOTAL).increment(statement.len() as u64);

        Ok(statement)
    }
}
