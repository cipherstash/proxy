use crate::error::Error;
use crate::prometheus::STATEMENTS_TOTAL;
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::parser::Parser;
use sqltk::parser::tokenizer::{Token, TokenWithSpan, Tokenizer};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

pub struct SqlParser;

impl SqlParser {
    /// Parse a SQL statement string into an SqlParser AST
    pub fn parse_statement(statement: &str) -> Result<ast::Statement, Error> {
        let statement = Parser::new(&DIALECT)
            .with_tokens_with_locations(reset_tokens(statement)?)
            .parse_statement()?;

        counter!(STATEMENTS_TOTAL).increment(1);

        Ok(statement)
    }

    /// Render the internal RESET ALL representation as valid PostgreSQL SQL.
    pub fn statement_sql(statement: &ast::Statement) -> String {
        if matches!(statement, ast::Statement::Set(ast::Set::SingleAssignment {
            variable, values, ..
        }) if variable.to_string().eq_ignore_ascii_case("ALL")
            && matches!(values.as_slice(), [ast::Expr::Identifier(value)]
                if value.quote_style.is_none() && value.value.eq_ignore_ascii_case("DEFAULT")))
        {
            "RESET ALL".to_owned()
        } else {
            statement.to_string()
        }
    }

    /// Parse a SQL String potentially containing multiple statements into parsed SqlParser AST
    pub fn parse_statements(statement: &str) -> Result<Vec<ast::Statement>, Error> {
        let statement = Parser::new(&DIALECT)
            .with_tokens_with_locations(reset_tokens(statement)?)
            .parse_statements()?;

        counter!(STATEMENTS_TOTAL).increment(statement.len() as u64);

        Ok(statement)
    }
}

/// sqltk has no RESET AST variant. Normalize only complete RESET commands at
/// token boundaries; quoted strings, comments and other statements stay intact.
/// The original wire SQL remains authoritative. SET ALL TO DEFAULT is an
/// internal sentinel rendered back through `statement_sql` when rewriting SQL,
/// so a user-written `SET ALL`, which PostgreSQL rejects, is rejected here too.
fn reset_tokens(sql: &str) -> Result<Vec<TokenWithSpan>, Error> {
    let tokens = Tokenizer::new(&DIALECT, sql)
        .tokenize_with_location()
        .map_err(|error| sqltk::parser::parser::ParserError::TokenizerError(error.to_string()))?;
    // Built only once a RESET is found; until then the tokens are unchanged.
    let mut rewritten: Option<Vec<TokenWithSpan>> = None;
    let mut offset = 0;
    for statement in tokens.split_inclusive(|token| token.token == Token::SemiColon) {
        let start = offset;
        offset += statement.len();
        let has_delimiter = statement
            .last()
            .is_some_and(|token| token.token == Token::SemiColon);
        let body = &statement[..statement.len() - usize::from(has_delimiter)];
        match leading_command(body) {
            LeadingCommand::SetAll => {
                return Err(sqltk::parser::parser::ParserError::ParserError(
                    "SET ALL is not valid PostgreSQL; use RESET ALL".to_owned(),
                )
                .into());
            }
            LeadingCommand::Reset {
                keyword,
                target_end,
                role,
            } => {
                let result = rewritten.get_or_insert_with(|| {
                    let mut result = Vec::with_capacity(tokens.len() + 2);
                    result.extend_from_slice(&tokens[..start]);
                    result
                });
                result.extend_from_slice(&body[..keyword]);
                result.push(TokenWithSpan::new(
                    Token::make_keyword("SET"),
                    body[keyword].span,
                ));
                result.extend_from_slice(&body[keyword + 1..=target_end]);
                // Generated tokens point to the original target, while every token
                // outside this RESET retains its original location, including in a batch.
                let target = &body[target_end];
                if role {
                    result.push(TokenWithSpan::new(Token::make_keyword("NONE"), target.span));
                } else {
                    result.push(TokenWithSpan::new(Token::make_keyword("TO"), target.span));
                    result.push(TokenWithSpan::new(
                        Token::make_keyword("DEFAULT"),
                        target.span,
                    ));
                }
                result.extend_from_slice(&statement[target_end + 1..]);
            }
            LeadingCommand::Other => {
                if let Some(result) = &mut rewritten {
                    result.extend_from_slice(statement);
                }
            }
        }
    }
    Ok(rewritten.unwrap_or(tokens))
}

enum LeadingCommand {
    /// `RESET name` or `RESET name.name`, by index of the RESET keyword and of
    /// the last target token within the statement. `RESET ROLE` sets `role`.
    Reset {
        keyword: usize,
        target_end: usize,
        role: bool,
    },
    /// `SET [SESSION | LOCAL] ALL ...`.
    SetAll,
    Other,
}

/// Classifies one statement from its first few significant tokens.
fn leading_command(body: &[TokenWithSpan]) -> LeadingCommand {
    let mut significant = body
        .iter()
        .enumerate()
        .filter(|(_, token)| !matches!(token.token, Token::Whitespace(_)))
        .map(|(index, token)| (index, &token.token));
    let Some((keyword, first)) = significant.next() else {
        return LeadingCommand::Other;
    };
    if is_keyword(first, "SET") {
        let mut next = significant.next().map(|(_, token)| token);
        if next.is_some_and(|token| is_keyword(token, "SESSION") || is_keyword(token, "LOCAL")) {
            next = significant.next().map(|(_, token)| token);
        }
        return if next.is_some_and(|token| is_keyword(token, "ALL")) {
            LeadingCommand::SetAll
        } else {
            LeadingCommand::Other
        };
    }
    if !is_keyword(first, "RESET") {
        return LeadingCommand::Other;
    }
    let (target_end, role) = match (
        significant.next(),
        significant.next(),
        significant.next(),
        significant.next(),
    ) {
        (Some((end, target @ Token::Word(_))), None, None, None) => {
            (end, is_keyword(target, "ROLE"))
        }
        (
            Some((_, Token::Word(_))),
            Some((_, Token::Period)),
            Some((end, Token::Word(_))),
            None,
        ) => (end, false),
        _ => return LeadingCommand::Other,
    };
    LeadingCommand::Reset {
        keyword,
        target_end,
        role,
    }
}

fn is_keyword(token: &Token, keyword: &str) -> bool {
    matches!(token, Token::Word(word)
        if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(keyword))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_sql_retains_original_error_locations_after_reset_normalization() {
        let invalid = "INSERT INTO encrypted id, encrypted_text VALUES ($1, $2)";
        for (sql, column) in [
            (invalid.to_owned(), 23),
            (format!("RESET ALL;{invalid}"), 33),
        ] {
            let error = SqlParser::parse_statements(&sql).unwrap_err();
            let Error::Mapping(crate::error::MappingError::InvalidSqlStatement(message)) = error
            else {
                panic!("expected invalid SQL error");
            };
            assert_eq!(message, format!(
                "sql parser error: Expected: SELECT, VALUES, or a subquery in the query body, found: id at Line: 1, Column: {column}"
            ));
        }
    }

    #[test]
    fn reset_normalization_is_limited_to_complete_commands() {
        let statements = SqlParser::parse_statements(
            "/* pool reset */ RESET ROLE; RESET ALL; RESET CIPHERSTASH.KEYSET_NAME;",
        )
        .unwrap();
        assert_eq!(statements.len(), 3);
        assert_eq!(SqlParser::statement_sql(&statements[1]), "RESET ALL");
        assert!(SqlParser::parse_statements("RESET ALL unexpected").is_err());
        let quoted = SqlParser::parse_statement("SELECT 'RESET ALL; RESET ROLE'").unwrap();
        assert_eq!(quoted.to_string(), "SELECT 'RESET ALL; RESET ROLE'");
    }

    #[test]
    fn lowercase_reset_all_is_normalised_and_rendered_as_reset_all() {
        let statement = SqlParser::parse_statement("reset all").unwrap();
        assert_eq!(SqlParser::statement_sql(&statement), "RESET ALL");
        assert!(matches!(
            crate::postgresql::context::settings::SettingChange::parse(&statement).unwrap(),
            Some(crate::postgresql::context::settings::SettingChange::ResetAll)
        ));
    }

    #[test]
    fn reset_role_becomes_set_role_none() {
        for sql in ["RESET ROLE", "reset role"] {
            let statement = SqlParser::parse_statement(sql).unwrap();
            assert_eq!(
                SqlParser::statement_sql(&statement),
                "SET ROLE NONE",
                "{sql}"
            );
        }
    }

    #[test]
    fn user_written_set_all_to_default_is_not_rendered_as_reset_all() {
        // PostgreSQL rejects `SET ALL TO DEFAULT` (ALL is reserved), so the
        // rewrite path must not turn it into a successful `RESET ALL`.
        let rendered = SqlParser::parse_statement("SET ALL TO DEFAULT")
            .ok()
            .map(|statement| SqlParser::statement_sql(&statement));
        assert_ne!(rendered.as_deref(), Some("RESET ALL"));
    }
}
