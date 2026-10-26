use crate::error::Error;
use crate::prometheus::STATEMENTS_TOTAL;
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::parser::{Parser, ParserError};
use sqltk::parser::tokenizer::{Span, Token, TokenWithSpan, Tokenizer, Whitespace};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

/// The setting name of the placeholder that carries a RESET with no SET equivalent.
/// Generated tokens have an empty span, which no token in client SQL has, so neither
/// this placeholder nor a generated `DEFAULT` can be mistaken for a client's own SET.
const RESET_PLACEHOLDER: &str = "__cipherstash_reset";

pub struct SqlParser;

impl SqlParser {
    /// Parse a SQL statement string into an SqlParser AST
    pub fn parse_statement(statement: &str) -> Result<ast::Statement, Error> {
        let tokens = reset_tokens(statement)?;
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

    /// Render a statement as PostgreSQL SQL, turning the internal representation of a
    /// RESET back into that RESET, so PostgreSQL answers it with the `RESET` command tag.
    pub fn statement_sql(statement: &ast::Statement) -> String {
        let ast::Statement::Set(ast::Set::SingleAssignment {
            variable, values, ..
        }) = statement
        else {
            return statement.to_string();
        };
        match values.as_slice() {
            [ast::Expr::Identifier(value)]
                if value.quote_style.is_none()
                    && value.value.eq_ignore_ascii_case("DEFAULT")
                    && variable.to_string().eq_ignore_ascii_case("ALL") =>
            {
                "RESET ALL".to_owned()
            }
            [ast::Expr::Identifier(value)] if value.span == Span::empty() => {
                format!("RESET {variable}")
            }
            [ast::Expr::Value(ast::ValueWithSpan {
                value: ast::Value::SingleQuotedString(reset),
                span,
            })] if *span == Span::empty()
                && matches!(variable.0.as_slice(), [ast::ObjectNamePart::Identifier(name)]
                    if name.span == Span::empty() && name.value == RESET_PLACEHOLDER) =>
            {
                reset.clone()
            }
            _ => statement.to_string(),
        }
    }

    /// Parse a SQL String potentially containing multiple statements into parsed SqlParser AST
    pub fn parse_statements(statement: &str) -> Result<Vec<ast::Statement>, Error> {
        let tokens = reset_tokens(statement)?;
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
}

/// sqltk has no RESET AST variant. Normalize only complete RESET commands at
/// token boundaries; quoted strings, comments and other statements stay intact.
/// The original wire SQL remains authoritative. SET ALL TO DEFAULT is an
/// internal sentinel rendered back through `statement_sql` when rewriting SQL,
/// so a user-written `SET ALL`, which PostgreSQL rejects, is rejected here too.
fn reset_tokens(sql: &str) -> Result<Vec<TokenWithSpan>, Error> {
    let tokens = Tokenizer::new(&DIALECT, sql)
        .tokenize_with_location()
        .map_err(|error| ParserError::TokenizerError(error.to_string()))?;
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
                return Err(ParserError::ParserError(
                    "SET ALL is not valid PostgreSQL; use RESET ALL".to_owned(),
                )
                .into());
            }
            LeadingCommand::Reset {
                keyword,
                target_end,
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
                // Every token outside this RESET retains its original location, including
                // in a batch. The generated DEFAULT marks this SET as a RESET.
                let target = &body[target_end];
                result.push(TokenWithSpan::new(Token::make_keyword("TO"), target.span));
                result.push(TokenWithSpan::new(
                    Token::make_keyword("DEFAULT"),
                    Span::empty(),
                ));
                result.extend_from_slice(&statement[target_end + 1..]);
            }
            LeadingCommand::ResetVerbatim { keyword, end } => {
                // No CipherStash setting is involved, so Proxy only needs a statement it
                // can parse. The placeholder carries the RESET's own SQL, which
                // `statement_sql` renders back unchanged.
                let result = rewritten.get_or_insert_with(|| {
                    let mut result = Vec::with_capacity(tokens.len() + 2);
                    result.extend_from_slice(&tokens[..start]);
                    result
                });
                let reset = body[keyword..=end]
                    .iter()
                    .map(|token| token.token.to_string())
                    .collect::<String>();
                result.extend_from_slice(&body[..keyword]);
                let space =
                    || TokenWithSpan::new(Token::Whitespace(Whitespace::Space), Span::empty());
                result.extend([
                    TokenWithSpan::new(Token::make_keyword("SET"), body[keyword].span),
                    space(),
                    TokenWithSpan::new(
                        Token::make_word(RESET_PLACEHOLDER, Some('"')),
                        Span::empty(),
                    ),
                    space(),
                    TokenWithSpan::new(Token::make_keyword("TO"), Span::empty()),
                    space(),
                    TokenWithSpan::new(Token::SingleQuotedString(reset), Span::empty()),
                ]);
                result.extend_from_slice(&statement[end + 1..]);
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

/// The SQL parser expects the copy data of a `COPY ... FROM STDIN` to follow it
/// in the SQL, as in a `psql` script: it refuses the statement without a `;`
/// after it, and reads the rest of the SQL after one as copy data. A client
/// sends the copy data in CopyData messages, so a statement after the `COPY` in
/// a Query is a statement, which Proxy must check.
fn contains_copy_from_stdin(tokens: &[TokenWithSpan]) -> bool {
    tokens
        .split(|token| token.token == Token::SemiColon)
        .any(is_copy_from_stdin)
}

/// Whether `statement` is a `COPY` with `FROM` followed by `STDIN` anywhere in
/// it. That also matches a query that reads a table named `stdin`, as in
/// `COPY (SELECT * FROM stdin) TO STDOUT`, which only makes Proxy parse the
/// Query one statement at a time, and so fails closed.
fn is_copy_from_stdin(statement: &[TokenWithSpan]) -> bool {
    let mut significant = statement
        .iter()
        .map(|token| &token.token)
        .filter(|token| !matches!(token, Token::Whitespace(_)));
    if !significant
        .next()
        .is_some_and(|token| is_keyword(token, "COPY"))
    {
        return false;
    }
    let mut after_from = false;
    for token in significant {
        if after_from && is_keyword(token, "STDIN") {
            return true;
        }
        after_from = is_keyword(token, "FROM");
    }
    false
}

/// Parses each statement on its own, ending each `COPY ... FROM STDIN` with the
/// `;` the SQL parser expects before its copy data, which is empty. A `;`
/// splits statements wherever it is outside a literal, quoted identifier or
/// comment, which is right for every statement but one with a body of several
/// statements, such as a `BEGIN ATOMIC` function, which then fails to parse.
fn parse_copy_from_stdin_statements(
    tokens: Vec<TokenWithSpan>,
) -> Result<Vec<ast::Statement>, ParserError> {
    let mut statements = vec![];
    for statement in tokens.split(|token| token.token == Token::SemiColon) {
        let mut statement = statement.to_vec();
        if is_copy_from_stdin(&statement) {
            statement.push(TokenWithSpan::wrap(Token::SemiColon));
        }
        statements.extend(
            Parser::new(&DIALECT)
                .with_tokens_with_locations(statement)
                .parse_statements()?,
        );
    }
    Ok(statements)
}

enum LeadingCommand {
    /// `RESET name` or `RESET name.name`, by index of the RESET keyword and of
    /// the last target token within the statement.
    Reset {
        keyword: usize,
        target_end: usize,
    },
    /// A RESET with no `SET ... TO DEFAULT` equivalent, by index of the RESET keyword
    /// and of its last token: `RESET ROLE`, `RESET TIME ZONE`,
    /// `RESET SESSION AUTHORIZATION` and `RESET TRANSACTION ISOLATION LEVEL`.
    ResetVerbatim {
        keyword: usize,
        end: usize,
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
    let target = significant.collect::<Vec<_>>();
    let keywords = |expected: &[&str]| {
        target.len() == expected.len()
            && target
                .iter()
                .zip(expected)
                .all(|((_, token), keyword)| is_keyword(token, keyword))
    };
    if keywords(&["ROLE"])
        || keywords(&["TIME", "ZONE"])
        || keywords(&["SESSION", "AUTHORIZATION"])
        || keywords(&["TRANSACTION", "ISOLATION", "LEVEL"])
    {
        let end = target.last().map(|(index, _)| *index).unwrap_or(keyword);
        return LeadingCommand::ResetVerbatim { keyword, end };
    }
    let target_end = match target.as_slice() {
        [(end, Token::Word(_))] => *end,
        [(_, Token::Word(_)), (_, Token::Period), (end, Token::Word(_))] => *end,
        _ => return LeadingCommand::Other,
    };
    LeadingCommand::Reset {
        keyword,
        target_end,
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
    fn every_reset_form_parses() {
        // Pool reset queries and drivers send these; PostgreSQL accepts each of them
        for sql in [
            "RESET ROLE",
            "RESET TIME ZONE",
            "RESET SESSION AUTHORIZATION",
            "RESET TRANSACTION ISOLATION LEVEL",
            "reset session authorization; reset time zone",
        ] {
            assert!(SqlParser::parse_statements(sql).is_ok(), "{sql}");
            if !sql.contains(';') {
                assert!(SqlParser::parse_statement(sql).is_ok(), "{sql}");
            }
        }
    }

    #[test]
    fn a_rewritten_batch_sends_each_reset_as_a_reset() {
        // PostgreSQL answers RESET with the `RESET` command tag, and SET with `SET`
        for (sql, expected) in [
            ("RESET ROLE", "RESET ROLE"),
            ("reset role", "reset role"),
            ("RESET search_path", "RESET search_path"),
            (
                "RESET CIPHERSTASH.KEYSET_NAME",
                "RESET CIPHERSTASH.KEYSET_NAME",
            ),
            (
                r#"RESET "cipherstash.keyset_id""#,
                r#"RESET "cipherstash.keyset_id""#,
            ),
            ("RESET TIME ZONE", "RESET TIME ZONE"),
            ("reset session authorization", "reset session authorization"),
            (
                "RESET TRANSACTION ISOLATION LEVEL",
                "RESET TRANSACTION ISOLATION LEVEL",
            ),
        ] {
            let statement = SqlParser::parse_statement(sql).unwrap();
            assert_eq!(SqlParser::statement_sql(&statement), expected, "{sql}");
        }
    }

    #[test]
    fn user_written_set_to_default_stays_a_set() {
        for sql in [
            "SET search_path TO DEFAULT",
            "SET CIPHERSTASH.KEYSET_NAME = DEFAULT",
        ] {
            let statement = SqlParser::parse_statement(sql).unwrap();
            assert!(
                SqlParser::statement_sql(&statement).starts_with("SET "),
                "{sql}"
            );
        }
    }

    #[test]
    fn a_reset_of_a_named_setting_still_resets_that_setting() {
        let statement = SqlParser::parse_statement("RESET CIPHERSTASH.KEYSET_NAME").unwrap();
        assert!(
            crate::postgresql::context::settings::SettingChange::parse(&statement)
                .unwrap()
                .is_some()
        );
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

    /// A client sends the copy data of a `COPY ... FROM STDIN` in CopyData
    /// messages, so the statements after it in the SQL are statements.
    #[test]
    fn statements_after_copy_from_stdin_are_statements() {
        let sql = |statements: Vec<ast::Statement>| {
            statements
                .iter()
                .map(|statement| match statement {
                    ast::Statement::Copy { values, .. } => {
                        assert!(values.is_empty());
                        "COPY".to_owned()
                    }
                    statement => SqlParser::statement_sql(statement),
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
        assert_eq!(
            sql(SqlParser::parse_statements("COPY users FROM STDIN; RESET ALL").unwrap()),
            vec!["COPY", "RESET ALL"]
        );
        assert!(SqlParser::parse_statements("COPY users FROM STDIN; SELECT 1 2").is_err());
    }
}
