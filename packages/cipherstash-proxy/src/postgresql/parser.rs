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
/// internal sentinel rendered back through `statement_sql` when rewriting SQL.
fn reset_tokens(sql: &str) -> Result<Vec<TokenWithSpan>, Error> {
    let tokens = Tokenizer::new(&DIALECT, sql)
        .tokenize_with_location()
        .map_err(|error| sqltk::parser::parser::ParserError::TokenizerError(error.to_string()))?;
    let mut result = Vec::with_capacity(tokens.len());
    for statement in tokens.split_inclusive(|token| token.token == Token::SemiColon) {
        let has_delimiter = statement
            .last()
            .is_some_and(|token| token.token == Token::SemiColon);
        let body = &statement[..statement.len() - usize::from(has_delimiter)];
        let significant: Vec<_> = body
            .iter()
            .filter(|token| !matches!(token.token, Token::Whitespace(_)))
            .collect();
        let reset = significant.first().is_some_and(|token| {
            matches!(&token.token, Token::Word(word)
            if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("RESET"))
        });
        let target = &significant[usize::from(!significant.is_empty())..];
        let target_tokens: Vec<_> = target.iter().map(|token| &token.token).collect();
        let valid_target = matches!(target_tokens.as_slice(), [Token::Word(_)])
            || matches!(
                target_tokens.as_slice(),
                [Token::Word(_), Token::Period, Token::Word(_)]
            );
        if reset && valid_target {
            result.push(TokenWithSpan::new(
                Token::make_keyword("SET"),
                significant[0].span,
            ));
            result.extend(target.iter().map(|token| (*token).clone()));
            // Generated tokens point to the original target, while every token
            // outside this RESET retains its original location, including in a batch.
            let span = target.last().unwrap().span;
            if matches!(target_tokens.as_slice(), [Token::Word(word)]
                if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("ROLE"))
            {
                result.push(TokenWithSpan::new(Token::make_keyword("NONE"), span));
            } else {
                result.push(TokenWithSpan::new(Token::make_keyword("TO"), span));
                result.push(TokenWithSpan::new(Token::make_keyword("DEFAULT"), span));
            }
        } else {
            result.extend_from_slice(body);
        }
        if has_delimiter {
            result.push(statement.last().unwrap().clone());
        }
    }
    Ok(result)
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
}
