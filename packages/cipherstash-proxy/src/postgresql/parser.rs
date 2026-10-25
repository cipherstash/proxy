use crate::error::Error;
use crate::prometheus::STATEMENTS_TOTAL;
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::keywords::Keyword;
use sqltk::parser::parser::Parser;
use sqltk::parser::parser::ParserError;
use sqltk::parser::tokenizer::{Token, TokenWithSpan, Tokenizer};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct SqlParser;

/// A `RESET` statement, which the SQL parser does not support
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetStatement {
    /// `RESET ALL`
    All,
    /// `RESET name`, with the name in lower case and its parts joined with `.`, as in
    /// `cipherstash.keyset_id`. PostgreSQL matches setting names without regard to case,
    /// quoted or not, and `"cipherstash.keyset_id"` names the same setting.
    Parameter(String),
}

impl SqlParser {
    /// Parse a SQL statement string into an SqlParser AST
    pub fn parse_statement(statement: &str) -> Result<ast::Statement, Error> {
        let tokens = tokenize(statement)?;
        let statement = if contains_copy_from_stdin(&tokens) {
            parse_copy_from_stdin_statements(tokens)?
                .into_iter()
                .next()
                .ok_or_else(|| ParserError::ParserError("Expected: a statement".to_owned()))?
        } else {
            Parser::new(&DIALECT)
                .with_tokens_with_locations(tokens)
                .parse_statement()?
        };

        counter!(STATEMENTS_TOTAL).increment(1);

        Ok(statement)
    }

    /// Parse a SQL String potentially containing multiple statements into parsed SqlParser AST
    pub fn parse_statements(statement: &str) -> Result<Vec<ast::Statement>, Error> {
        let tokens = tokenize(statement)?;
        let statement = if contains_copy_from_stdin(&tokens) {
            parse_copy_from_stdin_statements(tokens)?
        } else {
            Parser::new(&DIALECT)
                .with_tokens_with_locations(tokens)
                .parse_statements()?
        };

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
            let mut name = String::new();
            let mut words = 0;
            let mut all = false;
            let mut after_word = false;
            for token in tokens {
                match token {
                    Token::Word(word) => {
                        // Words not joined by a period, as in `RESET SESSION AUTHORIZATION`
                        if after_word {
                            name.push(' ');
                        }
                        all = word.keyword == Keyword::ALL && word.quote_style.is_none();
                        name.push_str(&word.value.to_lowercase());
                        words += 1;
                        after_word = true;
                    }
                    Token::Period => {
                        name.push('.');
                        after_word = false;
                    }
                    _ => return None,
                }
            }
            resets.push(match words {
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

fn tokenize(sql: &str) -> Result<Vec<TokenWithSpan>, ParserError> {
    Ok(Tokenizer::new(&DIALECT, sql).tokenize_with_location()?)
}

/// The SQL parser expects the copy data of a `COPY ... FROM STDIN` to follow it in the SQL, as
/// in a `psql` script. It refuses the statement without a `;` after `STDIN`, and parses the rest
/// of the SQL after one as copy data. A client sends the copy data in CopyData messages, so a
/// statement after the `COPY` in a Query is a statement, which Proxy must check.
fn contains_copy_from_stdin(tokens: &[TokenWithSpan]) -> bool {
    tokens
        .split(|token| token.token == Token::SemiColon)
        .any(is_copy_from_stdin)
}

fn is_copy_from_stdin(tokens: &[TokenWithSpan]) -> bool {
    let mut words = tokens
        .iter()
        .filter(|token| !matches!(token.token, Token::Whitespace(_)))
        .map(|token| match &token.token {
            Token::Word(word) => word.keyword,
            _ => Keyword::NoKeyword,
        });
    if words.next() != Some(Keyword::COPY) {
        return false;
    }
    let mut after_from = false;
    for keyword in words {
        if after_from && keyword == Keyword::STDIN {
            return true;
        }
        after_from = keyword == Keyword::FROM;
    }
    false
}

/// Parse each statement on its own, ending each `COPY ... FROM STDIN` with the `;` the SQL
/// parser expects before its copy data, which is empty. A `;` splits statements wherever it is
/// outside a literal, quoted identifier or comment, which is right for every statement but one
/// with a body of several statements, such as a `BEGIN ATOMIC` function, which then fails to
/// parse.
fn parse_copy_from_stdin_statements(
    tokens: Vec<TokenWithSpan>,
) -> Result<Vec<ast::Statement>, ParserError> {
    let mut statements = vec![];
    for tokens in tokens.split(|token| token.token == Token::SemiColon) {
        let mut tokens = tokens.to_vec();
        if is_copy_from_stdin(&tokens) {
            tokens.push(TokenWithSpan::wrap(Token::SemiColon));
        }
        statements.extend(
            Parser::new(&DIALECT)
                .with_tokens_with_locations(tokens)
                .parse_statements()?,
        );
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::{ResetStatement, SqlParser};

    fn parameter(name: &str) -> ResetStatement {
        ResetStatement::Parameter(name.to_owned())
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
            Some(vec![parameter("cipherstash.keyset_id")])
        );
        // A quoted name that includes the period names the same setting
        assert_eq!(
            SqlParser::parse_resets(r#"RESET "CIPHERSTASH.KEYSET_ID""#),
            Some(vec![parameter("cipherstash.keyset_id")])
        );
        assert_eq!(
            SqlParser::parse_resets("RESET search_path; /* comment */ RESET ALL ;"),
            Some(vec![parameter("search_path"), ResetStatement::All])
        );
        assert_eq!(
            SqlParser::parse_resets("RESET SESSION AUTHORIZATION"),
            Some(vec![parameter("session authorization")])
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

    /// The SQL parser has no `RESET` statement, so `parse_resets` is needed
    #[test]
    fn the_sql_parser_rejects_reset() {
        assert!(SqlParser::parse_statements("RESET ALL").is_err());
    }

    /// A client sends the copy data of a `COPY ... FROM STDIN` in CopyData messages, so the
    /// statements after it in the SQL are statements
    #[test]
    fn statements_after_copy_from_stdin_are_statements() {
        let sql = |statements: Vec<sqltk::parser::ast::Statement>| {
            statements
                .iter()
                .map(|statement| match statement {
                    sqltk::parser::ast::Statement::Copy { values, .. } => {
                        assert!(values.is_empty());
                        "COPY".to_owned()
                    }
                    statement => statement.to_string(),
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            sql(SqlParser::parse_statements("COPY users (email) FROM STDIN").unwrap()),
            vec!["COPY"]
        );
        assert_eq!(
            sql(
                SqlParser::parse_statements("COPY first FROM STDIN; COPY second FROM STDIN")
                    .unwrap()
            ),
            vec!["COPY", "COPY"]
        );
        assert_eq!(
            sql(SqlParser::parse_statements(
                "copy users from stdin with (format csv);\n INSERT INTO t VALUES ('a;b'); "
            )
            .unwrap()),
            vec!["COPY", "INSERT INTO t VALUES ('a;b')"]
        );
        assert_eq!(
            sql(vec![SqlParser::parse_statement(
                "/* bulk */ COPY users FROM STDIN CSV"
            )
            .unwrap()]),
            vec!["COPY"]
        );
        assert!(SqlParser::parse_statements("COPY users FROM STDIN; SELECT 1 2").is_err());
    }
}
