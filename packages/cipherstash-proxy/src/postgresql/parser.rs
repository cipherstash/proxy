use crate::error::Error;
use crate::prometheus::STATEMENTS_TOTAL;
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::keywords::Keyword;
use sqltk::parser::parser::Parser;
use sqltk::parser::tokenizer::{Token, Tokenizer};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct SqlParser;

/// A `RESET` statement, which the SQL parser does not support
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetStatement {
    /// `RESET ALL`
    All,
    /// `RESET name`, with each word of the name in lower case. PostgreSQL matches setting
    /// names without regard to case, quoted or not.
    Parameter(Vec<String>),
}

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

    /// Recognise SQL made up only of `RESET` statements.
    ///
    /// Returns `None` if `sql` contains any other statement, or none at all, so that it is
    /// handled as before. PostgreSQL validates the names.
    pub fn parse_resets(sql: &str) -> Option<Vec<ResetStatement>> {
        let tokens = Tokenizer::new(&DIALECT, sql).tokenize().ok()?;
        let mut resets = vec![];
        for statement in tokens.split(|token| *token == Token::SemiColon) {
            let mut tokens = statement
                .iter()
                .filter(|token| !matches!(token, Token::Whitespace(_)));
            match tokens.next() {
                None => continue,
                Some(Token::Word(word)) if word.keyword == Keyword::RESET => (),
                Some(_) => return None,
            }
            let mut name = vec![];
            let mut all = false;
            for token in tokens {
                match token {
                    Token::Word(word) => {
                        all = word.keyword == Keyword::ALL && word.quote_style.is_none();
                        name.push(word.value.to_lowercase());
                    }
                    Token::Period => (),
                    _ => return None,
                }
            }
            resets.push(match name.len() {
                0 => return None,
                1 if all => ResetStatement::All,
                _ => ResetStatement::Parameter(name),
            });
        }
        if resets.is_empty() {
            return None;
        }
        counter!(STATEMENTS_TOTAL).increment(resets.len() as u64);
        Some(resets)
    }
}

#[cfg(test)]
mod tests {
    use super::{ResetStatement, SqlParser};

    fn parameter(name: &[&str]) -> ResetStatement {
        ResetStatement::Parameter(name.iter().map(|part| part.to_string()).collect())
    }

    #[test]
    fn reset_statements_are_recognised() {
        assert_eq!(
            SqlParser::parse_resets("RESET ALL"),
            Some(vec![ResetStatement::All])
        );
        assert_eq!(
            SqlParser::parse_resets("reset all;"),
            Some(vec![ResetStatement::All])
        );
        // Setting names are matched without regard to case, as PostgreSQL does
        assert_eq!(
            SqlParser::parse_resets(r#"RESET "CipherStash".KEYSET_ID"#),
            Some(vec![parameter(&["cipherstash", "keyset_id"])])
        );
        assert_eq!(
            SqlParser::parse_resets("RESET search_path; /* comment */ RESET ALL ;"),
            Some(vec![parameter(&["search_path"]), ResetStatement::All])
        );
        assert_eq!(
            SqlParser::parse_resets("RESET SESSION AUTHORIZATION"),
            Some(vec![parameter(&["session", "authorization"])])
        );
    }

    #[test]
    fn sql_with_other_statements_is_not_a_reset() {
        assert_eq!(SqlParser::parse_resets(""), None);
        assert_eq!(SqlParser::parse_resets(";"), None);
        assert_eq!(SqlParser::parse_resets("DISCARD ALL"), None);
        assert_eq!(SqlParser::parse_resets("RESET ALL; SELECT 1"), None);
        assert_eq!(SqlParser::parse_resets("RESET"), None);
        assert_eq!(SqlParser::parse_resets("RESET 1"), None);
        assert_eq!(SqlParser::parse_resets("SELECT 'RESET ALL'"), None);
    }
}
