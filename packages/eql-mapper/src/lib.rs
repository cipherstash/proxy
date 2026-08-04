//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod dep;
mod display_helpers;
mod eql_mapper;
mod importer;
mod inference;
mod iterator_ext;
mod json_value_selector;
mod model;
mod param;
mod param_plan;
mod query_operands;
mod renumber_params;
mod scope_tracker;
mod transformation_rules;
mod type_checked_statement;

#[cfg(test)]
mod test_helpers;

pub use display_helpers::*;
pub use eql_mapper::*;
pub use json_value_selector::*;
pub use model::*;
pub use param::*;
pub use param_plan::*;
pub use query_operands::*;
pub use type_checked_statement::*;
pub use unifier::{
    Array, AssociatedType, DomainIdentity, EqlTerm, EqlTermVariant, EqlTrait, EqlTraits, EqlValue,
    NativeValue, Projection, ProjectionColumn, SetOf, TableColumn, TokenType, Type, Value,
};

pub(crate) use dep::*;
pub(crate) use inference::*;
pub(crate) use renumber_params::*;
pub(crate) use scope_tracker::*;
pub(crate) use transformation_rules::*;

#[cfg(test)]
mod test {
    use super::{test_helpers::*, type_check};
    use crate::{
        projection, schema, test_helpers,
        unifier::{
            EqlTerm, EqlTrait, EqlTraits, EqlValue, InstantiateType, NativeValue, Projection,
            ProjectionColumn, Type, Value,
        },
        JsonSelectorSegment, JsonSelectorSource, OutputParamSource, Param, Schema, TableColumn,
        TableResolver, TypeCheckedStatement,
    };
    use eql_mapper_macros::concrete_ty;
    use pretty_assertions::assert_eq;
    use sqltk::{
        parser::ast::{self as ast, Ident, Statement},
        AsNodeKey, NodeKey,
    };
    use std::{collections::HashMap, sync::Arc};
    use tracing::error;

    fn resolver(schema: Schema) -> Arc<TableResolver> {
        Arc::new(TableResolver::new_fixed(schema.into()))
    }

    #[test]
    fn basic() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.projection,
                    concrete_ty!({ Native(users.email) as email } as Projection)
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn basic_with_value() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                    first_name,
                }
            }
        });

        let statement = parse("select email from users WHERE email = 'hello@cipherstash.com'");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.projection,
                    concrete_ty! {{EQL(users.email: Eq) as email} as Projection}
                );

                assert_eq!(
                    typed.literals,
                    vec![(
                        EqlTerm::Full(EqlValue::with_canonical_identity(
                            TableColumn {
                                table: id("users"),
                                column: id("email"),
                            },
                            EqlTraits::from(EqlTrait::Eq)
                        ),),
                        &ast::Value::SingleQuotedString("hello@cipherstash.com".into()),
                    )]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn like_on_token_match_column_type_checks() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email LIKE 'a%'");
        assert!(
            type_check(schema, &statement).is_ok(),
            "LIKE on a TokenMatch column should type check"
        );
    }

    #[test]
    fn like_rewrites_to_match_term() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email LIKE 'a%'");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM users WHERE eql_v3.match_term(email) @> eql_v3.match_term('ENCRYPTED'::JSONB::eql_v3.query_text_match)"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn ord_ore_column_rewrites_to_ord_term_ore() {
        // The explicit EQL("<domain>") form pins a block-ORE ordering domain, so
        // the rewrite must select ord_term_ore (not ord_term) and the query twin
        // must be query_integer_ord_ore.
        let schema = resolver(schema! {
            tables: {
                events: {
                    id,
                    seq (EQL("eql_v3_integer_ord_ore"): Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM events WHERE seq > 5");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM events WHERE eql_v3.ord_term_ore(seq) > eql_v3.ord_term_ore('ENCRYPTED'::JSONB::eql_v3.query_integer_ord_ore)"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn at_at_rewrites_to_match_term() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email @@ 'a'");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM users WHERE eql_v3.match_term(email) @> eql_v3.match_term('ENCRYPTED'::JSONB::eql_v3.query_text_match)"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn like_on_non_match_encrypted_column_is_rejected() {
        // Regression: LIKE used to unify to Native and bypass capability checking.
        // An encrypted column that only implements Eq must not accept LIKE.
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email LIKE 'a%'");
        assert!(
            type_check(schema, &statement).is_err(),
            "LIKE on a non-TokenMatch encrypted column should be a capability error"
        );
    }

    #[test]
    fn ilike_rewrites_to_match_term() {
        // ILIKE takes the same match arm as LIKE (RewriteEqlMatchOps handles both),
        // so it must rewrite to the identical match_term form.
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email ILIKE 'a%'");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM users WHERE eql_v3.match_term(email) @> eql_v3.match_term('ENCRYPTED'::JSONB::eql_v3.query_text_match)"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn not_like_rewrites_to_negated_match_term() {
        // The `negated` arm wraps the containment in `NOT (...)` — otherwise
        // untested (only the positive LIKE/ILIKE/@@ forms had rewrite coverage).
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email NOT LIKE 'a%'");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM users WHERE NOT (eql_v3.match_term(email) @> eql_v3.match_term('ENCRYPTED'::JSONB::eql_v3.query_text_match))"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn not_ilike_rewrites_to_negated_match_term() {
        // NOT ILIKE takes the same negated match arm as NOT LIKE.
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: TokenMatch),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email NOT ILIKE 'a%'");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "SELECT id FROM users WHERE NOT (eql_v3.match_term(email) @> eql_v3.match_term('ENCRYPTED'::JSONB::eql_v3.query_text_match))"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn native_like_still_type_checks() {
        // Regression: routing LIKE/ILIKE through the TokenMatch-bounded rule must
        // not regress plain LIKE on a native (non-encrypted) column — Native
        // satisfies all bounds, so `WHERE native_col LIKE 'x'` still type checks.
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email LIKE 'a%'");
        assert!(
            type_check(schema, &statement).is_ok(),
            "LIKE on a native column should still type check"
        );
    }

    #[test]
    fn update_set_casts_stored_value() {
        // ADR-0003's second stored-value context: an UPDATE SET on an encrypted
        // column casts the assigned literal to the column domain, exactly like
        // INSERT. Only INSERT had cast-target rewrite coverage before this.
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse("UPDATE employees SET salary = 20000 WHERE id = 123");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed) => assert_eq!(
                transformed.to_string(),
                "UPDATE employees SET salary = 'ENCRYPTED'::JSONB::public.eql_v3_text WHERE id = 123"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        };
    }

    #[test]
    fn insert_with_value() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users (id, email) VALUES (42, 'hello@cipherstash.com')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert!(typed.literals.contains(&(
                    EqlTerm::Full(EqlValue::with_canonical_identity(
                        TableColumn {
                            table: id("users"),
                            column: id("email")
                        },
                        EqlTraits::default()
                    )),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into()),
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn insert_with_values_no_explicit_columns() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users VALUES (42, 'hello@cipherstash.com', 'James')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert!(typed.literals.contains(&(
                    EqlTerm::Full(EqlValue::with_canonical_identity(
                        TableColumn {
                            table: id("users"),
                            column: id("email")
                        },
                        EqlTraits::default()
                    )),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into()),
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn insert_with_values_no_explicit_columns_but_has_default() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement =
            parse("INSERT INTO users VALUES (default, 'hello@cipherstash.com', 'James')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert!(typed.literals.contains(&(
                    EqlTerm::Full(EqlValue::with_canonical_identity(
                        TableColumn {
                            table: id("users"),
                            column: id("email")
                        },
                        EqlTraits::default()
                    )),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into()),
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn basic_with_placeholder() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users WHERE id = $1");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let v: Value = Value::Native(NativeValue(Some(TableColumn {
                    table: id("users"),
                    column: id("id"),
                })));

                let (_, value) = typed.params.first().unwrap();

                assert_eq!(value, &v);

                assert_eq!(
                    typed.projection,
                    projection![(NATIVE(users.email) as email)]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_placeholder() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
            }
        });

        let statement =
            parse("select id, email, first_name from users WHERE email = $1 AND first_name = $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Native(NativeValue(Some(TableColumn {
                    table: id("users"),
                    column: id("email"),
                })));

                let b = Value::Native(NativeValue(Some(TableColumn {
                    table: id("users"),
                    column: id("first_name"),
                })));

                assert_eq!(typed.params, vec![(Param(1), a), (Param(2), b)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (NATIVE(users.email) as email),
                        (NATIVE(users.first_name) as first_name)
                    ]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_instances_of_placeholder() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
            }
        });

        let statement =
            parse("select id, email, first_name from users WHERE email = $1 OR first_name = $1");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Native(NativeValue(Some(TableColumn {
                    table: id("users"),
                    column: id("email"),
                })));

                assert_eq!(typed.params, vec![(Param(1), a)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (NATIVE(users.email) as email),
                        (NATIVE(users.first_name) as first_name)
                    ]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_multiple_tables() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                    first_name,
                }
                todo_lists: {
                    id,
                    name,
                    owner_id,
                    created_at,
                    updated_at,
                }
            }
        });

        let statement = parse(
            r#"
            select
                u.email
            from
                users as u
            inner
                join todo_lists as tl on tl.owner_id = u.id
            ;
            "#,
        );

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert_eq!(typed.projection, projection![(EQL(users.email) as email)])
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_subquery() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
                todo_lists: {
                    id,
                    name,
                    owner_id,
                    created_at,
                    updated_at,
                }
                todo_list_items: {
                    id,
                    description (EQL),
                    owner_id,
                    created_at,
                    updated_at,
                }
            }
        });

        let statement = parse(
            r#"
                select
                    u.id as user_id,
                    tli.id as todo_list_item_id,
                    tli.description as todo_list_item_description
                from
                    users as u
                inner join (
                    select
                        id,
                        owner_id,
                        description
                    from
                        todo_list_items
                ) as tli on tli.owner_id = u.id;
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("{}", err),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(users.id) as user_id),
                (NATIVE(todo_list_items.id) as todo_list_item_id),
                (EQL(todo_list_items.description) as todo_list_item_description)
            ]
        );
    }

    #[test]
    fn wildcard_expansion() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                }
                todo_lists: {
                    id,
                    owner_id,
                    secret (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    u.*,
                    tl.*
                from
                    users as u
                inner join todo_lists as tl on tl.owner_id = u.id
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(users.id) as id),
                (EQL(users.email) as email),
                (NATIVE(todo_lists.id) as id),
                (NATIVE(todo_lists.owner_id) as owner_id),
                (EQL(todo_lists.secret) as secret)
            ]
        );
    }

    #[test]
    fn wildcard_expansion_2() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                }
                todo_lists: {
                    id,
                    owner_id,
                    secret (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select * from (
                    select
                        u.*,
                        tl.*
                    from
                        users as u
                    inner join todo_lists as tl on tl.owner_id = u.id
                )
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(users.id) as id),
                (EQL(users.email) as email),
                (NATIVE(todo_lists.id) as id),
                (NATIVE(todo_lists.owner_id) as owner_id),
                (EQL(todo_lists.secret) as secret)
            ]
        );
    }

    #[test]
    fn select_with_multiple_placeholder_and_wildcard_expansion() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    // `=` is equality, so both columns have to declare it.
                    email (EQL: Eq),
                    first_name (EQL: Eq),
                }
            }
        });

        let statement = parse("select * from users WHERE email = $1 AND first_name = $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("users"),
                        column: id("email"),
                    },
                    EqlTraits::from(EqlTrait::Eq),
                )));

                let b = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("users"),
                        column: id("first_name"),
                    },
                    EqlTraits::from(EqlTrait::Eq),
                )));

                assert_eq!(typed.params, vec![(Param(1), a,), (Param(2), b,)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.email: Eq) as email),
                        (EQL(users.first_name: Eq) as first_name)
                    ]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_placeholder_boolean_operators_and_wildcard_expansion() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    salary (EQL: Ord),
                    age (EQL: Ord),
                }
            }
        });

        let statement = parse("select * from users WHERE salary > $1 AND age <= $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("users"),
                        column: id("salary"),
                    },
                    EqlTraits::from(EqlTrait::Ord),
                )));

                let b = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("users"),
                        column: id("age"),
                    },
                    EqlTraits::from(EqlTrait::Ord),
                )));

                assert_eq!(typed.params, vec![(Param(1), a,), (Param(2), b,)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.salary: Ord) as salary),
                        (EQL(users.age: Ord) as age)
                    ]
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn correlated_subquery() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    salary
                from
                    employees
                where
                    salary > (select salary from employees where first_name = 'Alice')
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (EQL(employees.salary: Ord) as salary)
            ]
        );
    }

    #[test]
    fn window_function() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    department_name,
                    salary,
                    rank() over (partition by department_name order by salary desc)
                from
                   employees
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary: Ord) as salary),
                (NATIVE as rank)
            ]
        );
    }

    #[test]
    fn window_function_with_forward_reference() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    department_name,
                    salary,
                    rank() over w
                from
                   employees
                window w AS (partition BY department_name order by salary desc);
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary: Ord) as salary),
                (NATIVE as rank)
            ]
        );
    }

    #[test]
    fn common_table_expressions() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                with salaries_by_department as (
                    select
                        first_name,
                        last_name,
                        department_name,
                        salary,
                        rank() over w
                    from
                    employees
                    window w AS (partition BY department_name order by salary desc)
                )
                select * from salaries_by_department
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => {
                panic!("type check failed: {err:#?}")
            }
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary: Ord) as salary),
                (NATIVE as rank)
            ]
        );
    }

    #[test]
    fn cte_tables_can_be_resolved_in_subqueries() {
        let schema = resolver(schema! {
            tables: {
                source_table: {
                    id,
                }

                dest_table: {
                    id,
                }
            }
        });

        let statement = parse(
            "
            WITH fd AS ( SELECT id FROM source_table )
            INSERT INTO dest_table ( id )
            SELECT id FROM fd RETURNING id
        ",
        );

        type_check(schema, &statement).unwrap();
    }

    #[test]
    fn aggregates() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    department,
                    age,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    max(age),
                    min(salary)
                from employees
                group by department
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.age) as max),
                (EQL(employees.salary: Ord) as min)
            ]
        );
    }

    #[test]
    fn insert() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                insert into employees (name, department, age, salary)
                    values ('Alice', 'Engineering', 28, 180000)
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(typed.projection, Projection(vec![]));
    }

    #[test]
    fn insert_with_returning_clause() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                insert into employees (name, department, age, salary)
                    values ('Alice', 'Engineering', 28, 180000)
                    returning *
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ]
        );
    }

    #[test]
    fn update() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                update employees set name = 'Alice', salary = 18000 where id = 123
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(typed.projection, Projection(vec![]));
    }

    #[test]
    fn update_with_returning_clause() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                update employees set name = 'Alice', salary = 18000 where id = 123 returning *
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ]
        );
    }

    /// In `UPDATE t1 SET x = ... FROM t2` the assignment target must resolve
    /// against the table being updated, not through the lexical scope. The
    /// scope also contains the `FROM` relations, so a same-named column there
    /// used to make the target spuriously ambiguous (and could shadow it).
    /// Here both tables have an `email` column; the assignment must get
    /// `users.email` — the encrypted one.
    #[test]
    fn update_assignment_resolves_against_target_table_not_from_relation() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
                aux: {
                    id,
                    email,
                }
            }
        });

        let statement = parse("UPDATE users SET email = $1 FROM aux WHERE users.id = aux.id");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        let target = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
            TableColumn {
                table: id("users"),
                column: id("email"),
            },
            EqlTraits::from(EqlTrait::Eq),
        )));

        assert_eq!(typed.params, vec![(Param(1), target)]);
        assert_eq!(typed.projection, Projection(vec![]));
    }

    /// Proxy loads its schema from the database with *quoted* column idents
    /// (`Ident::with_quote('"', ..)`) behind an editable resolver, while SQL
    /// usually spells the same columns unquoted. A type identity derived from
    /// an assignment target must still unify with one derived from the scope,
    /// so the resolver has to return the schema's canonical idents rather than
    /// echo the caller's spelling. With the caller's spelling,
    /// `UPDATE t SET c = $1 WHERE c = $1` pinned the same param to
    /// `EQL(t."c")` and `EQL(t.c)` and failed with "cannot unify EQL terms".
    #[test]
    fn update_reused_param_unifies_against_quoted_schema_idents() {
        let eq = EqlTraits::from(EqlTrait::Eq);

        let mut schema = Schema::new("public");
        let mut table = crate::model::Table::new(Ident::new("encrypted"));
        table.add_column(Arc::new(crate::model::Column::native(Ident::with_quote(
            '"', "id",
        ))));
        table.add_column(Arc::new(crate::model::Column::eql(
            Ident::with_quote('"', "encrypted_text"),
            eq,
            crate::unifier::DomainIdentity::canonical(crate::unifier::TokenType::Text, eq),
        )));
        schema.add_table(table);

        // The editable resolver is the one Proxy uses at runtime; it resolves
        // through `SchemaDelta`, not `Schema`.
        let resolver = Arc::new(TableResolver::new_editable(Arc::new(schema)));

        let statement = parse("UPDATE encrypted SET encrypted_text = $1 WHERE encrypted_text = $1");

        let typed = match type_check(resolver, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        // The param's identity is the canonical (quoted) schema spelling.
        let target = Value::Eql(EqlTerm::Full(EqlValue::with_canonical_identity(
            TableColumn {
                table: id("encrypted"),
                column: Ident::with_quote('"', "encrypted_text"),
            },
            eq,
        )));

        assert_eq!(typed.params, vec![(Param(1), target)]);
    }

    /// The row-count expressions in `LIMIT`/`OFFSET` can never be encrypted,
    /// so placeholders there must be pinned to `Native` at inference time.
    /// Previously they were left as unconstrained type variables and only
    /// resolved to `Native` by the late unresolved-value fallback in
    /// `Unifier::resolve_unresolved_value_nodes` — this pins the guarantee
    /// where the clause is inferred instead of relying on that fallback.
    #[test]
    fn limit_and_offset_placeholders_infer_native() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users LIMIT $1 OFFSET $2");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        assert_eq!(
            typed.params,
            vec![
                (Param(1), Value::Native(NativeValue(None))),
                (Param(2), Value::Native(NativeValue(None))),
            ]
        );
    }

    /// Same as `limit_and_offset_placeholders_infer_native`, but for the
    /// quantity in a `FETCH FIRST n ROWS ONLY` clause.
    #[test]
    fn fetch_first_placeholder_infers_native() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users FETCH FIRST $1 ROWS ONLY");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        assert_eq!(
            typed.params,
            vec![(Param(1), Value::Native(NativeValue(None)))]
        );
    }

    /// Because `LIMIT` is pinned to `Native` at inference time, an encrypted
    /// value can no longer flow into it silently — the mapper refuses the
    /// statement instead of forwarding SQL that the database would reject
    /// (or worse, that would leak a ciphertext into a row count).
    #[test]
    fn encrypted_column_in_limit_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users LIMIT email");

        type_check(schema, &statement)
            .expect_err("an encrypted column must not type check as a LIMIT row count");
    }

    /// A statement variant with no inference rule must fail closed with an
    /// error stating the invariant, not traverse without constraining the
    /// statement's top-level type. (`requires_type_check` never admits
    /// `TRUNCATE`, so this can only be reached by calling `type_check`
    /// directly — but if `requires_type_check` is ever widened without a
    /// matching inference rule, this is the error that makes it loud.)
    #[test]
    fn statement_without_inference_rule_fails_closed() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                }
            }
        });

        let statement = parse("TRUNCATE TABLE users");

        match type_check(schema, &statement) {
            Ok(_) => panic!("expected type check to fail"),
            Err(err) => assert_eq!(
                err.to_string(),
                format!(
                    "type inference has no rule for statement `{statement}`; \
                     `requires_type_check` admits a statement variant that \
                     `InferType<'_, Statement>` does not handle"
                )
            ),
        }
    }

    /// `WHERE`, `HAVING` and join `ON` conditions are boolean expressions and
    /// booleans are always native, so a bare placeholder condition is pinned to
    /// `Native` where the clause is inferred — it must not depend on the late
    /// unresolved-value fallback (which is now fail-closed).
    #[test]
    fn where_condition_placeholder_infers_native() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE $1");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        assert_eq!(
            typed.params,
            vec![(Param(1), Value::Native(NativeValue(None)))]
        );
    }

    /// Same as `where_condition_placeholder_infers_native`, but for a literal
    /// condition in `HAVING`.
    #[test]
    fn having_constant_condition_type_checks() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                }
            }
        });

        let statement = parse("SELECT id FROM users GROUP BY id HAVING true");

        if let Err(err) = type_check(schema, &statement) {
            panic!("type check failed: {err}");
        }
    }

    /// Same as `where_condition_placeholder_infers_native`, but for a literal
    /// join `ON` condition (common in lateral joins: `JOIN ... ON true`).
    #[test]
    fn join_on_constant_condition_type_checks() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                }
                aux: {
                    id,
                }
            }
        });

        let statement = parse("SELECT u.id FROM users AS u JOIN aux AS a ON true");

        if let Err(err) = type_check(schema, &statement) {
            panic!("type check failed: {err}");
        }
    }

    /// Because a `WHERE` condition is pinned to `Native`, an encrypted column
    /// cannot itself be the condition — the mapper refuses the statement
    /// instead of forwarding SQL that would compare against the raw jsonb
    /// payload.
    #[test]
    fn encrypted_column_as_bare_where_condition_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM users WHERE email");

        type_check(schema, &statement)
            .expect_err("an encrypted column must not type check as a WHERE condition");
    }

    /// An `ORDER BY` ordinal after a set operation cannot be resolved against a
    /// single `SELECT`'s projection, but the literal is still a plain constant
    /// to the database and is pinned to `Native` where the clause is inferred.
    #[test]
    fn order_by_ordinal_after_set_operation_type_checks() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                }
            }
        });

        let statement = parse("SELECT id FROM users UNION ALL SELECT id FROM users ORDER BY 1");

        if let Err(err) = type_check(schema, &statement) {
            panic!("type check failed: {err}");
        }
    }

    /// A literal in a derived-table column that the outer query never
    /// references relates to nothing — its type escapes only through the
    /// subquery's projection, so it resolves to `Native` rather than being
    /// treated as an inference gap.
    #[test]
    fn unreferenced_derived_table_value_column_type_checks() {
        let schema = resolver(schema! {
            tables: {
                users: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM (SELECT id, 'lit' AS unused FROM users) AS sub");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err}"),
        };

        assert_eq!(typed.projection, projection![(NATIVE(users.id) as id)]);
    }

    #[test]
    fn delete() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                delete from employees where salary > 200000
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(typed.projection, Projection(vec![]));
    }

    #[test]
    fn delete_with_returning_clause() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                delete from employees where salary > 200000 returning *
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary: Ord) as salary)
            ]
        );
    }

    #[test]
    fn select_with_literal_cast_as_encrypted() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select * from employees where salary > 200000
            "#,
        );

        let typed = match type_check(schema.clone(), &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.literals,
            vec![(
                EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("employees"),
                        column: id("salary")
                    },
                    EqlTraits::from(EqlTrait::Ord)
                ),),
                &ast::Value::Number(200000.into(), false),
            )]
        );

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT * FROM employees WHERE eql_v3.ord_term(salary) > eql_v3.ord_term('ENCRYPTED'::JSONB::eql_v3.query_text_ord)"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        };
    }

    #[test]
    fn insert_with_literal_cast_as_encrypted() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                insert into employees (salary) values (20000)
            "#,
        );

        let typed = match type_check(schema.clone(), &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.literals,
            vec![(
                EqlTerm::Full(EqlValue::with_canonical_identity(
                    TableColumn {
                        table: id("employees"),
                        column: id("salary")
                    },
                    EqlTraits::default()
                )),
                &ast::Value::Number(20000.into(), false)
            )]
        );

        match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )])) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "INSERT INTO employees (salary) VALUES ('ENCRYPTED'::JSONB::public.eql_v3_text)"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        };
    }

    #[test]
    fn pathologically_complex_sql_statement() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    department_id,
                    name,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
                select * from
                (select min(salary) as min_salary from employees) as x
                inner join (
                    (
                        select salary as y from employees
                            where salary < (select min(foo) from (
                                select salary as foo from employees
                            )
                        )
                    )
                    -- `union all`, not `union`: deduplicating here would compare
                    -- the encrypted payloads rather than the salaries, which the
                    -- type checker now refuses.
                    union all
                    (
                        select salary as y from employees
                            where salary >= (select min(max(foo)) from (
                                select salary as foo from employees
                            )
                        )
                    )
                ) as holy_joins_batman on x.min_salary = holy_joins_batman.y
                inner join employees as e on (e.salary = holy_joins_batman.y)
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert_eq!(
            typed.projection,
            projection![
                (EQL(employees.salary: Ord) as min_salary),
                (EQL(employees.salary: Ord) as y),
                (NATIVE(employees.id) as id),
                (NATIVE(employees.department_id) as department_id),
                (NATIVE(employees.name) as name),
                (EQL(employees.salary: Ord) as salary)
            ]
        );
    }

    #[test]
    fn literals_or_param_placeholders_in_outermost_projection() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: { }
        });

        // PROBLEM: the literal `1` is not a value from a table column and it has not been used with a function or
        // operator - which means its type has not been constrained, hence why its type is still an unresolved type
        // variable.
        //
        // The rule: if any column of the outermost projection contains an unresolved type variable AND if that type
        // variable is associated with a `Expr::Value(_)` then it is safe to resolve it to `NativeValue(None)`.

        // All of these statements should have the same projection type (after flattening & ignoring aliases):
        // e.g. `projection![(NATIVE)]`

        let projection_type = |statement: &Statement| {
            ignore_aliases(&type_check(schema.clone(), statement).unwrap().projection)
        };

        assert_transitive_eq(&[
            projection_type(&parse("select 'lit'")),
            projection_type(&parse("select x from (select 'lit' as x)")),
            projection_type(&parse("select * from (select 'lit')")),
            projection_type(&parse("select * from (select 'lit' as t)")),
            projection_type(&parse("select $1")),
            projection_type(&parse("select t from (select $1 as t)")),
            projection_type(&parse("select * from (select $1)")),
            Projection(vec![ProjectionColumn {
                alias: None,
                ty: Arc::new(Type::Value(Value::Native(NativeValue(None)))),
            }]),
        ]);
    }

    #[test]
    fn where_true() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                }
            }
        });

        let statement = parse(
            r#"
                select id from employees where true;
            "#,
        );
        type_check(schema, &statement).unwrap();
    }

    #[test]
    fn function_with_literal() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select upper('x'), salary from employees;
            "#,
        );
        let typed = type_check(schema, &statement).unwrap();

        error!("{:?}", typed.projection);
        assert_eq!(
            typed.projection,
            projection![(NATIVE as upper), (EQL(employees.salary) as salary)]
        );
    }

    #[test]
    fn function_with_wildcard() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    // `group by salary` is equality, so the column has to
                    // declare it.
                    salary (EQL: Eq),
                }
            }
        });

        let statement = parse(
            r#"
                select count(*), salary from employees group by salary;
            "#,
        );
        let typed = type_check(schema, &statement).unwrap();

        assert_eq!(
            typed.projection,
            projection![(NATIVE as count), (EQL(employees.salary: Eq) as salary)]
        );
    }

    #[test]
    fn function_with_column_and_literal() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select concat(name, 'x'), salary from employees;
            "#,
        );
        let typed = type_check(schema, &statement).unwrap();

        assert_eq!(
            typed.projection,
            projection![(NATIVE as concat), (EQL(employees.salary) as salary)]
        );
    }

    #[test]
    fn function_with_param() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select concat(name, $1), salary from employees;
            "#,
        );

        let typed = type_check(schema, &statement).unwrap();

        let a = Value::Native(NativeValue(None));

        assert_eq!(typed.params, vec![(Param(1), a)]);

        assert_eq!(
            typed.projection,
            projection![(NATIVE as concat), (EQL(employees.salary) as salary)]
        );
    }

    #[test]
    fn function_with_eql_column_and_literal() {
        // init_tracing();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name (EQL),
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select concat(name, 'x'), salary from employees;
            "#,
        );

        type_check(schema, &statement)
            .expect_err("eql columns in functions should be a type error");
    }

    #[test]
    fn modify_aggregate_when_eql_column_affected_by_group_by_of_other_column() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    department,
                    // `min`/`max` are ordering, so the column has to declare it.
                    salary (EQL: Ord),
                }
            }
        });

        let statement =
            parse("SELECT min(salary), max(salary), department FROM employees GROUP BY department");

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(HashMap::new()) {
                    Ok(statement) => assert_eq!(
                        statement.to_string(),
                        "SELECT eql_v3.min(salary), eql_v3.max(salary), department FROM employees GROUP BY department".to_string()
                    ),
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_params_cast_as_encrypted() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    eql_col (EQL: Eq),
                    native_col,
                }
            }
        });

        let statement = parse(
            "
            SELECT * FROM employees WHERE eql_col = $1 AND native_col = $2;
        ",
        );

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                            statement.to_string(),
                            "SELECT * FROM employees WHERE eql_v3.eq_term(eql_col) = eql_v3.eq_term($1::JSONB::eql_v3.query_text_eq) AND native_col = $2"
                        );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn rewrite_standard_sql_fns_on_eql_types() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    eql_col (EQL: JsonLike),
                    native_col,
                }
            }
        });

        let statement = parse(
            "
            SELECT
                jsonb_path_exists(eql_col, '$.another-secret'),
                jsonb_path_query(eql_col, '$.secret'),
                jsonb_path_query(native_col, '$.not-secret')
            FROM employees
        ",
        );

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(test_helpers::dummy_encrypted_json_selector(
                    &statement,
                    vec![
                        ast::Value::SingleQuotedString("$.secret".into()),
                        ast::Value::SingleQuotedString("$.another-secret".into()),
                    ],
                )) {
                    Ok(statement) => {
                        assert_eq!(
                            statement.to_string(),
                            "SELECT \
                            eql_v3.jsonb_path_exists(eql_col, '<encrypted-selector($.another-secret)>'), \
                            eql_v3.jsonb_path_query(eql_col, '<encrypted-selector($.secret)>'), \
                            jsonb_path_query(native_col, '$.not-secret') \
                            FROM employees"
                        );
                    }
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn supports_named_arrays() {
        let schema = resolver(schema! {
            tables: {
            }
        });

        let statement = parse("SELECT ARRAY[1, 2, 3]");

        type_check(schema, &statement).expect("named arrays should be supported");
    }

    #[test]
    fn jsonb_operator_arrow() {
        // init_tracing();
        test_jsonb_operator("->");
    }

    #[test]
    fn jsonb_operator_long_arrow() {
        test_jsonb_operator("->>");
    }

    #[test]
    #[ignore = "? is unimplemented"]
    fn jsonb_operator_hash_at_at() {
        test_jsonb_operator("@@");
    }

    #[test]
    #[ignore = "@? is unimplemented"]
    fn jsonb_operator_at_question() {
        test_jsonb_operator("@?");
    }

    #[test]
    #[ignore = "? is unimplemented"]
    fn jsonb_operator_question() {
        test_jsonb_operator("?");
    }

    #[test]
    #[ignore = "?& is unimplemented"]
    fn jsonb_operator_question_and() {
        test_jsonb_operator("?&");
    }

    #[test]
    #[ignore = "?| is unimplemented"]
    fn jsonb_operator_question_pipe() {
        test_jsonb_operator("?|");
    }

    #[test]
    fn jsonb_operator_at_arrow() {
        test_jsonb_operator("@>");
    }

    #[test]
    fn jsonb_operator_arrow_at() {
        test_jsonb_operator("<@");
    }

    #[test]
    fn jsonb_function_jsonb_path_query() {
        test_jsonb_function(
            "jsonb_path_query",
            vec![
                ast::Expr::Identifier(Ident::new("notes")),
                ast::Expr::Value(ast::ValueWithSpan {
                    value: ast::Value::SingleQuotedString("$.medications".to_owned()),
                    span: sqltk::parser::tokenizer::Span::empty(),
                }),
            ],
        );
    }

    fn test_jsonb_function(fn_name: &str, args: Vec<ast::Expr>) {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike),
                }
            }
        });

        let args_in = args
            .iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        let statement = parse(&format!(
            "SELECT id, {fn_name}({args_in}) AS meds FROM patients"
        ));

        let args_encrypted = args
            .iter()
            .map(|expr| match expr {
                ast::Expr::Identifier(ident) => ident.to_string(),
                ast::Expr::Value(ast::ValueWithSpan {
                    value: ast::Value::SingleQuotedString(s),
                    span: _,
                }) => {
                    // A jsonb_path_query selector is emitted as bare encrypted-selector
                    // text (eql_v3.jsonb_path_query(json, text)), not a jsonb cast.
                    format!("'<encrypted-selector({s})>'")
                }
                _ => panic!("unsupported expr type in test util"),
            })
            .collect::<Vec<String>>()
            .join(", ");

        let mut encrypted_literals: HashMap<NodeKey<'_>, ast::Value> = HashMap::new();

        for arg in args.iter() {
            if let ast::Expr::Value(ast::ValueWithSpan { value, .. }) = arg {
                encrypted_literals.extend(test_helpers::dummy_encrypted_json_selector(
                    &statement,
                    vec![value.clone()],
                ));
            }
        }

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(encrypted_literals) {
                Ok(statement) => {
                    let rewritten_fn_name = format!("eql_v3.{fn_name}");
                    assert_eq!(
                        statement.to_string(),
                        format!(
                            "SELECT id, {}({}) AS meds FROM patients",
                            rewritten_fn_name, args_encrypted
                        )
                    )
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    fn test_jsonb_operator(op: &str) {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse(&format!(
            "SELECT id, notes {op} 'medications' AS meds FROM patients",
        ));

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(test_helpers::dummy_encrypted_json_selector(
                    &statement,
                    vec![ast::Value::SingleQuotedString("medications".to_owned())],
                )) {
                    Ok(statement) => {
                        let expected = match op {
                            "@>" => "SELECT id, eql_v3.jsonb_contains(notes, '<encrypted-selector(medications)>'::JSONB::public.eql_v3_text_search) AS meds FROM patients".to_string(),
                            "<@" => "SELECT id, eql_v3.jsonb_contained_by(notes, '<encrypted-selector(medications)>'::JSONB::public.eql_v3_text_search) AS meds FROM patients".to_string(),
                            // -> / ->> field access: functionalised to eql_v3."->"/"->>",
                            // with the field selector passed as encrypted text.
                            "->" => "SELECT id, eql_v3.\"->\"(notes, '<encrypted-selector(medications)>') AS meds FROM patients".to_string(),
                            "->>" => "SELECT id, eql_v3.\"->>\"(notes, '<encrypted-selector(medications)>') AS meds FROM patients".to_string(),
                            _ => format!("SELECT id, notes {op} '<encrypted-selector(medications)>'::JSONB::public.eql_v3_text_search AS meds FROM patients"),
                        };
                        assert_eq!(statement.to_string(), expected)
                    }
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn jsonb_array_function() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse(
            "SELECT id FROM patients WHERE eql_v3.jsonb_array(notes) @> eql_v3.jsonb_array(notes)",
        );

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v3.jsonb_array: {err}"),
        }
    }

    #[test]
    fn jsonb_contains_function() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, notes)");

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v3.jsonb_contains: {err}"),
        }
    }

    #[test]
    fn jsonb_contained_by_function() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement =
            parse("SELECT id FROM patients WHERE eql_v3.jsonb_contained_by(notes, notes)");

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v3.jsonb_contained_by: {err}"),
        }
    }

    #[test]
    fn eql_v3_jsonb_contains_with_param() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, $1)");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // Verify param was inferred as EQL type
        assert!(typed.params_contain_eql(), "param $1 should be EQL type");

        // Verify transformation output - function passes through, param gets cast
        match typed.transform(HashMap::new()) {
            Ok(statement) => assert_eq!(
                statement.to_string(),
                "SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, $1::JSONB::public.eql_v3_text_search)"
            ),
            Err(err) => panic!("transformation failed: {err}"),
        }
    }

    #[test]
    fn containment_operator_transforms_to_function() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE notes @> $1");

        let typed =
            type_check(schema, &statement).expect("type check failed for containment operator");
        let transformed = typed
            .transform(HashMap::new())
            .expect("transformation failed");
        let sql = transformed.to_string();

        // Verify function call exists
        assert!(
            sql.contains("eql_v3.jsonb_contains"),
            "Expected @> to be transformed to eql_v3.jsonb_contains, got: {sql}"
        );

        // CRITICAL: Verify the parameter is cast to enable GIN index usage
        // The cast ::JSONB::public.eql_v3_text_search is required for GIN indexes to work
        assert!(
            sql.contains("::JSONB::public.eql_v3_text_search") || sql.contains("::jsonb::public.eql_v3_text_search"),
            "Expected parameter to be cast as ::JSONB::public.eql_v3_text_search for GIN index support, got: {sql}"
        );
    }

    #[test]
    fn contained_by_operator_transforms_to_function() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE $1 <@ notes");

        let typed =
            type_check(schema, &statement).expect("type check failed for contained_by operator");
        let transformed = typed
            .transform(HashMap::new())
            .expect("transformation failed");
        let sql = transformed.to_string();

        // Verify function call exists
        assert!(
            sql.contains("eql_v3.jsonb_contained_by"),
            "Expected <@ to be transformed to eql_v3.jsonb_contained_by, got: {sql}"
        );

        // CRITICAL: Verify the parameter is cast to enable GIN index usage
        assert!(
            sql.contains("::JSONB::public.eql_v3_text_search") || sql.contains("::jsonb::public.eql_v3_text_search"),
            "Expected parameter to be cast as ::JSONB::public.eql_v3_text_search for GIN index support, got: {sql}"
        );
    }

    #[test]
    fn explain_statement_transforms_containment_operator() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        // EXPLAIN wraps the inner SELECT - transformation should still apply
        let statement = parse("EXPLAIN SELECT id FROM patients WHERE notes @> $1");

        let typed = type_check(schema, &statement)
            .expect("type check failed for EXPLAIN with containment operator");
        let transformed = typed
            .transform(HashMap::new())
            .expect("transformation failed");
        let sql = transformed.to_string();

        // Verify EXPLAIN is preserved
        assert!(
            sql.starts_with("EXPLAIN"),
            "Expected EXPLAIN prefix preserved, got: {sql}"
        );

        // Verify function call exists inside the EXPLAIN
        assert!(
            sql.contains("eql_v3.jsonb_contains"),
            "Expected @> inside EXPLAIN to be transformed to eql_v3.jsonb_contains, got: {sql}"
        );
    }

    #[test]
    fn eql_term_partial_is_unified_with_eql_term_whole() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    email (EQL: Eq),
                }
            }
        });

        // let statement = parse(
        //     "SELECT id, email FROM patients WHERE email = 'alice@example.com'"
        // );

        let statement = parse(
            "
            SELECT id, email FROM patients AS p
            INNER JOIN (
                SELECT 'alice@example.com' AS selector
            ) AS selectors
            WHERE p.email = selectors.selector
        ",
        );

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        assert_eq!(
            typed.projection,
            projection![(NATIVE(patients.id) as id), (EQL(patients.email: Eq) as email)]
        );
    }

    #[test]
    fn select_with_multiple_joins() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                workspace: {
                    id,
                    resource_id,
                }
                workspace_entity: {
                    id,
                    workspace_id,
                    entity_id,
                }
                entity: {
                    id,
                    resource_id,
                    deleted_at,
                }
            }
        });

        let statement = parse(
            r#"
                SELECT
                    ARRAY_REMOVE(
                        ARRAY_AGG(e.resource_id), NULL
                    )::text [] AS entity_resource_ids,
                    workspace.*
                FROM workspace
                LEFT JOIN workspace_entity AS we ON workspace.id = we.workspace_id
                LEFT JOIN entity AS e ON we.entity_id = e.id
                WHERE
                    workspace.resource_id = $1
                    AND e.deleted_at IS NULL
                GROUP BY workspace.id;
            "#,
        );

        match type_check(schema.clone(), &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE as entity_resource_ids),
                        (NATIVE(workspace.id) as id),
                        (NATIVE(workspace.resource_id) as resource_id)
                    ]
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }

        let statement = parse(
            r#"
                SELECT
                    ARRAY_REMOVE(
                        ARRAY_AGG(e.resource_id), NULL
                    )::text [] AS entity_resource_ids,
                    workspace.id,
                    workspace.resource_id
                FROM workspace
                LEFT JOIN workspace_entity AS we ON workspace.id = we.workspace_id
                LEFT JOIN entity AS e ON we.entity_id = e.id
                WHERE
                    workspace.id < $1
                    AND (
                        CARDINALITY($2::text []) = 0
                        OR e.resource_id = ANY($3::text [])
                    )
                GROUP BY workspace.id
                ORDER BY workspace.id DESC
                LIMIT
                    $4
                    OFFSET $5;
            "#,
        );

        match type_check(schema.clone(), &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE as entity_resource_ids),
                        (NATIVE(workspace.id) as id),
                        (NATIVE(workspace.resource_id) as resource_id)
                    ]
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }

        let statement = parse(
            r#"
                SELECT COUNT(*) FROM workspace
                JOIN workspace_entity AS we ON workspace.id = we.workspace_id
                JOIN entity AS e on e.id = we.entity_id
                WHERE e.resource_id = ANY($1::varchar[]);
            "#,
        );

        match type_check(schema.clone(), &statement) {
            Ok(typed) => {
                assert_eq!(typed.projection, projection![(NATIVE as COUNT)])
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// A schema with one encrypted JSON column, for the value-selector tests.
    ///
    /// The domain is spelled out: value-selector fusion keys off the column's
    /// *token* type being `Json`, and the macro's default synthesises a `text`
    /// token regardless of the `JsonLike` capability.
    fn json_eq_schema() -> Arc<TableResolver> {
        resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL("eql_v3_json_search"): JsonLike + Contain),
                }
            }
        })
    }

    /// `col -> $1 = $2` fuses both operands into ONE containment needle: the
    /// field access is discarded and the two input params become a single
    /// output param, renumbered `$1`.
    #[test]
    fn json_field_eq_params_rewrites_to_containment() {
        let statement = parse("SELECT id FROM patients WHERE notes -> $1 = $2");

        let typed = type_check(json_eq_schema(), &statement).unwrap();
        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, $1::JSONB::eql_v3.query_json)"
        );

        // Two input params, one output param, derived from both.
        assert_eq!(transformed.params.len(), 1);
        assert_eq!(
            transformed.params.outputs()[0].source,
            OutputParamSource::JsonValueSelector {
                path: JsonSelectorSource::param(Param(1)),
                value: Param(2),
            }
        );
        assert!(!transformed.params.is_identity());
    }

    /// The `->>` and `jsonb_path_query_first` spellings are the same access and
    /// rewrite identically.
    #[test]
    fn json_field_eq_alternate_spellings_rewrite_to_containment() {
        for access in ["notes ->> $1", "jsonb_path_query_first(notes, $1)"] {
            let statement = parse(&format!("SELECT id FROM patients WHERE {access} = $2"));
            let typed = type_check(json_eq_schema(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                "SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, $1::JSONB::eql_v3.query_json)",
                "unexpected rewrite for `{access}`"
            );
        }
    }

    /// Params around a fusion are renumbered to close the gap it leaves, and the
    /// plan records where each surviving output param came from.
    #[test]
    fn json_field_eq_renumbers_surrounding_params() {
        let statement =
            parse("SELECT id FROM patients WHERE id = $1 AND notes -> $2 = $3 AND id <> $4");

        let typed = type_check(json_eq_schema(), &statement).unwrap();
        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM patients WHERE id = $1 AND eql_v3.jsonb_contains(notes, $2::JSONB::eql_v3.query_json) AND id <> $3"
        );

        let outputs = transformed.params.outputs();
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].source, OutputParamSource::Input(Param(1)));
        assert_eq!(
            outputs[1].source,
            OutputParamSource::JsonValueSelector {
                path: JsonSelectorSource::param(Param(2)),
                value: Param(3),
            }
        );
        assert_eq!(outputs[2].source, OutputParamSource::Input(Param(4)));
    }

    /// A statement whose params the rewrite leaves alone reports an identity
    /// plan, so the proxy can keep binding by position.
    #[test]
    fn ordinary_params_produce_an_identity_plan() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    name (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE id = $1 AND name = $2");
        let typed = type_check(schema, &statement).unwrap();
        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(transformed.params.len(), 2);
        assert!(transformed.params.is_identity());
    }

    /// Both halves as literals: the selector text is captured inline at
    /// type-check time, and the selector literal vanishes from the output.
    #[test]
    fn json_field_eq_literals_rewrites_to_containment() {
        let statement =
            parse("SELECT id FROM patients WHERE notes -> 'medications' = '\"aspirin\"'");

        let typed = type_check(json_eq_schema(), &statement).unwrap();

        assert_eq!(
            typed
                .json_value_selectors
                .for_literal(&ast::Value::SingleQuotedString("\"aspirin\"".to_owned())),
            None,
            "for_literal is keyed by node identity, not by value"
        );

        // Both literals are encrypted operands; only the value one survives the
        // rewrite, so the selector's replacement is simply never placed.
        let encrypted = HashMap::from_iter([
            (
                test_helpers::get_node_key_of_json_selector(
                    &statement,
                    &ast::Value::SingleQuotedString("medications".to_owned()),
                ),
                ast::Value::SingleQuotedString("<selector>".to_owned()),
            ),
            (
                test_helpers::get_node_key_of_json_selector(
                    &statement,
                    &ast::Value::SingleQuotedString("\"aspirin\"".to_owned()),
                ),
                ast::Value::SingleQuotedString("<needle>".to_owned()),
            ),
        ]);

        assert_eq!(
            typed.transform(encrypted).unwrap().to_string(),
            "SELECT id FROM patients WHERE eql_v3.jsonb_contains(notes, '<needle>'::JSONB::eql_v3.query_json)"
        );
    }

    /// `<>` is containment negated.
    #[test]
    fn json_field_not_eq_rewrites_to_negated_containment() {
        let statement = parse("SELECT id FROM patients WHERE notes -> $1 <> $2");

        let typed = type_check(json_eq_schema(), &statement).unwrap();

        assert_eq!(
            typed.transform(HashMap::new()).unwrap().to_string(),
            "SELECT id FROM patients WHERE NOT (eql_v3.jsonb_contains(notes, $1::JSONB::eql_v3.query_json))"
        );
    }

    /// Equality on the whole encrypted JSON column is document equality, NOT a
    /// field access — it must keep its ordinary term rewrite. Guards against the
    /// value-selector fusion swallowing every `=` on a JSON column.
    #[test]
    fn json_column_eq_is_not_value_selector_containment() {
        let statement = parse("SELECT id FROM patients WHERE notes = $1");

        let typed = type_check(json_eq_schema(), &statement).unwrap();

        assert_eq!(typed.json_value_selectors.for_param(Param(1)), None);
        assert!(typed.json_value_selectors.is_empty());

        let sql = typed.transform(HashMap::new()).unwrap().to_string();
        assert!(
            !sql.contains("jsonb_contains"),
            "whole-column equality must not become containment, got: {sql}"
        );
    }

    /// `ORDER BY` on an encrypted column must order by its ordering TERM. A bare
    /// `ORDER BY col` compares the jsonb payloads, whose first field is the
    /// randomised ciphertext — silently returning rows in an arbitrary order
    /// that differs on every insert.
    #[test]
    fn order_by_encrypted_column_uses_ord_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (order_by, expected) in [
            ("salary", "eql_v3.ord_term(salary)"),
            ("salary DESC", "eql_v3.ord_term(salary) DESC"),
            (
                "salary ASC NULLS FIRST",
                "eql_v3.ord_term(salary) ASC NULLS FIRST",
            ),
            (
                "employees.salary DESC NULLS LAST",
                "eql_v3.ord_term(employees.salary) DESC NULLS LAST",
            ),
            // A native column is left alone; a mixed list rewrites only the
            // encrypted term.
            ("id", "id"),
            ("id, salary", "id, eql_v3.ord_term(salary)"),
        ] {
            let statement = parse(&format!("SELECT id FROM employees ORDER BY {order_by}"));
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                format!("SELECT id FROM employees ORDER BY {expected}"),
                "unexpected rewrite for `ORDER BY {order_by}`"
            );
        }
    }

    /// `SELECT DISTINCT` ordered by an encrypted column has to be restructured.
    ///
    /// `RewriteEqlOrderBy` must order by `ord_term(col)`, but PostgreSQL requires
    /// every `ORDER BY` expression under `DISTINCT` to appear in the select list
    /// — and the term does not. The select is pushed into a subquery that also
    /// projects the term, and the (non-DISTINCT) outer query orders by it.
    #[test]
    fn distinct_order_by_encrypted_column_wraps_in_subquery() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected) in [
            // The ordering term is projected by the subquery, not by the client's
            // result set.
            (
                "SELECT DISTINCT id, salary FROM employees ORDER BY salary",
                "SELECT __eql_col_0 AS id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1, \
                 eql_v3.ord_term(salary) AS __eql_ord_0 FROM employees) AS __eql_distinct \
                 ORDER BY __eql_ord_0",
            ),
            // Sort options ride along with the hoisted term.
            (
                "SELECT DISTINCT id, salary FROM employees ORDER BY salary DESC NULLS FIRST",
                "SELECT __eql_col_0 AS id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1, \
                 eql_v3.ord_term(salary) AS __eql_ord_0 FROM employees) AS __eql_distinct \
                 ORDER BY __eql_ord_0 DESC NULLS FIRST",
            ),
            // A native term in the same ORDER BY is carried through as a
            // reference to the column the subquery already projects.
            (
                "SELECT DISTINCT id, salary FROM employees ORDER BY id, salary",
                "SELECT __eql_col_0 AS id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1, \
                 eql_v3.ord_term(salary) AS __eql_ord_0 FROM employees) AS __eql_distinct \
                 ORDER BY __eql_col_0, __eql_ord_0",
            ),
            // An ordinal still refers to the right column: the outer projection
            // preserves both the order and the count.
            (
                "SELECT DISTINCT id, salary FROM employees ORDER BY 1, salary",
                "SELECT __eql_col_0 AS id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1, \
                 eql_v3.ord_term(salary) AS __eql_ord_0 FROM employees) AS __eql_distinct \
                 ORDER BY 1, __eql_ord_0",
            ),
            // The client's column names survive the round trip.
            (
                "SELECT DISTINCT id AS employee_id, salary FROM employees ORDER BY salary",
                "SELECT __eql_col_0 AS employee_id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1, \
                 eql_v3.ord_term(salary) AS __eql_ord_0 FROM employees) AS __eql_distinct \
                 ORDER BY __eql_ord_0",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                expected.split_whitespace().collect::<Vec<_>>().join(" "),
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// The subquery wrapping is applied only where the `DISTINCT` and
    /// `ORDER BY` constraints actually collide.
    #[test]
    fn distinct_order_by_wraps_only_when_needed() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected) in [
            // Ordered by a native column, but the projection still dedupes an
            // encrypted one — so the DISTINCT ON that produces has to be kept
            // away from the ORDER BY, which means wrapping.
            (
                "SELECT DISTINCT id, salary FROM employees ORDER BY id",
                "SELECT __eql_col_0 AS id, __eql_col_1 AS salary \
                 FROM (SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id AS __eql_col_0, \
                 salary AS __eql_col_1 FROM employees) AS __eql_distinct \
                 ORDER BY __eql_col_0",
            ),
            // Ordered by an encrypted column, but not DISTINCT — the term is
            // allowed to sit in ORDER BY on its own.
            (
                "SELECT id, salary FROM employees ORDER BY salary",
                "SELECT id, salary FROM employees ORDER BY eql_v3.ord_term(salary)",
            ),
            // DISTINCT with no ORDER BY: dedupes on the term in place, no
            // wrapping needed.
            (
                "SELECT DISTINCT id, salary FROM employees",
                "SELECT DISTINCT ON (id, eql_v3.ord_term(salary)) id, salary FROM employees",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// `SELECT DISTINCT` on an encrypted column must deduplicate on the column's
    /// equality term. A bare `DISTINCT` compares whole jsonb payloads, whose
    /// ciphertext is randomised per row, so equal plaintexts never collapse and
    /// `DISTINCT` silently returns duplicates.
    #[test]
    fn distinct_on_encrypted_column_dedupes_on_eq_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    email (EQL("eql_v3_text_search"): Eq + Ord + TokenMatch),
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected) in [
            // A domain that stores `hm` keys on `eq_term`.
            (
                "SELECT DISTINCT email FROM employees",
                "SELECT DISTINCT ON (eql_v3.eq_term(email)) email FROM employees",
            ),
            // An ord-only domain stores no `hm`, so equality falls back to the
            // ordering term — the same fallback `=` uses.
            (
                "SELECT DISTINCT salary FROM employees",
                "SELECT DISTINCT ON (eql_v3.ord_term(salary)) salary FROM employees",
            ),
            // A plaintext column keys on itself.
            (
                "SELECT DISTINCT id, email FROM employees",
                "SELECT DISTINCT ON (id, eql_v3.eq_term(email)) id, email FROM employees",
            ),
            // No encrypted column in the projection: left alone entirely.
            (
                "SELECT DISTINCT id FROM employees",
                "SELECT DISTINCT id FROM employees",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// Deduplication is equality, so `DISTINCT` on a column whose domain carries
    /// no equality term is a capability error — caught during type checking,
    /// before any rewrite is attempted.
    #[test]
    fn distinct_on_a_column_without_equality_is_a_type_error() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    // Storage-only: a two-value column leaks its distribution
                    // under any index, so v3 gives boolean no searchable terms.
                    active (EQL("eql_v3_boolean")),
                }
            }
        });

        for input in [
            "SELECT DISTINCT active FROM employees",
            "SELECT DISTINCT ON (active) id FROM employees",
        ] {
            let statement = parse(input);

            assert!(
                type_check(schema.clone(), &statement).is_err(),
                "expected `{input}` to fail type checking"
            );
        }
    }

    /// An aggregate over a grouped encrypted column must not be lifted through
    /// `grouped_value`. `MIN(col)` resolves to the same `EqlValue` as `col`, so
    /// a naive match wraps it as `grouped_value(eql_v3.min(col))` — an aggregate
    /// inside an aggregate, which PostgreSQL rejects. An aggregate already
    /// yields one value per group.
    #[test]
    fn group_by_does_not_lift_aggregate_projections() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected) in [
            // The aggregate is retargeted but NOT wrapped.
            (
                "SELECT MIN(salary) FROM employees GROUP BY salary",
                "SELECT eql_v3.MIN(salary) FROM employees GROUP BY eql_v3.ord_term(salary)",
            ),
            (
                "SELECT MAX(salary) FROM employees GROUP BY salary",
                "SELECT eql_v3.MAX(salary) FROM employees GROUP BY eql_v3.ord_term(salary)",
            ),
            // A direct projection of the grouped column still is wrapped — that
            // is the case `grouped_value` exists for.
            (
                "SELECT salary, COUNT(*) FROM employees GROUP BY salary",
                "SELECT eql_v3.grouped_value(salary) AS salary, COUNT(*) FROM employees \
                 GROUP BY eql_v3.ord_term(salary)",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                expected.split_whitespace().collect::<Vec<_>>().join(" "),
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// `SELECT *` hides the projected columns, so a grouped encrypted column
    /// cannot be lifted through `grouped_value` — and left alone it is no longer
    /// functionally dependent on the rewritten key. Rejected by name rather than
    /// left to fail as a bare PostgreSQL error.
    #[test]
    fn group_by_rejects_wildcard_projections() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        let statement = parse("SELECT * FROM employees GROUP BY id, salary");
        let typed = type_check(schema.clone(), &statement).unwrap();

        let err = typed
            .transform(HashMap::new())
            .expect_err("SELECT * with GROUP BY on an encrypted column should be rejected");

        assert!(
            err.to_string().contains("SELECT *"),
            "unexpected error: {err}"
        );

        // Grouping only on plaintext columns leaves the wildcard alone.
        let statement = parse("SELECT * FROM employees GROUP BY id");
        let typed = type_check(schema, &statement).unwrap();
        assert_eq!(
            typed.transform(HashMap::new()).unwrap().to_string(),
            "SELECT * FROM employees GROUP BY id"
        );
    }

    /// The wrapper's synthetic names must not be shadowed by a user column of
    /// the same name — the outer `ORDER BY` would resolve to the user's column
    /// instead of the projected ordering term.
    #[test]
    fn distinct_order_by_synthetic_names_avoid_user_columns() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    __eql_col_0,
                    __eql_ord_0,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        let statement = parse(
            "SELECT DISTINCT __eql_col_0, __eql_ord_0, salary FROM employees ORDER BY salary",
        );
        let typed = type_check(schema, &statement).unwrap();
        let sql = typed.transform(HashMap::new()).unwrap().to_string();

        // The prefix grew past the user's columns, so the names it generates
        // cannot be the ones the query already mentions.
        assert!(
            sql.contains("___eql_col_0"),
            "expected a lengthened prefix, got: {sql}"
        );
        assert!(
            sql.contains("___eql_ord_0"),
            "expected a lengthened ordering name, got: {sql}"
        );
        // The user's own columns are still projected under their own names.
        assert!(
            sql.contains("AS __eql_col_0") && sql.contains("AS __eql_ord_0"),
            "user columns lost their names: {sql}"
        );
    }

    /// `@@` is symmetric in PostgreSQL, so the encrypted column may be written
    /// on either side. Both spellings must produce the same containment, with
    /// the pattern — not the column — as the encrypted needle.
    #[test]
    fn match_op_normalises_reversed_operands() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    email (EQL("eql_v3_text_search"): Eq + Ord + TokenMatch),
                }
            }
        });

        let expected = "SELECT id FROM employees WHERE eql_v3.match_term(email) @> \
                        eql_v3.match_term('<ENCRYPTED>'::JSONB::eql_v3.query_text_search)";

        for sql in [
            "SELECT id FROM employees WHERE email @@ 'a%'",
            // Reversed: previously emitted match_term('a%') @> match_term(email),
            // with the pattern left unencrypted.
            "SELECT id FROM employees WHERE 'a%' @@ email",
        ] {
            let statement = parse(sql);
            let typed = type_check(schema.clone(), &statement).unwrap();

            // One literal — the pattern — must be encrypted, whichever side it
            // was written on.
            assert_eq!(1, typed.literals.len(), "unexpected literals for `{sql}`");

            let encrypted = typed
                .literals
                .iter()
                .map(|(_, v)| {
                    (
                        NodeKey::new(*v),
                        ast::Value::SingleQuotedString("<ENCRYPTED>".to_string()),
                    )
                })
                .collect::<HashMap<_, _>>();

            assert_eq!(
                typed.transform(encrypted).unwrap().to_string(),
                expected.split_whitespace().collect::<Vec<_>>().join(" "),
                "unexpected rewrite for `{sql}`"
            );
        }
    }

    /// `SELECT DISTINCT *` must not be exempt from the equality-term keying:
    /// the wildcard hides the encrypted columns, but they are still what
    /// `DISTINCT` deduplicates on.
    #[test]
    fn distinct_wildcard_is_expanded_and_keyed() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    email (EQL("eql_v3_text_search"): Eq + Ord + TokenMatch),
                }
            }
        });

        let statement = parse("SELECT DISTINCT * FROM employees");
        let typed = type_check(schema.clone(), &statement).unwrap();

        assert_eq!(
            typed.transform(HashMap::new()).unwrap().to_string(),
            "SELECT DISTINCT ON (employees.id, eql_v3.eq_term(employees.email)) \
             employees.id, employees.email FROM employees"
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    /// And a wildcard hiding a column that cannot be deduplicated is a
    /// capability error, not a silent no-op.
    #[test]
    fn distinct_wildcard_over_a_column_without_equality_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    active (EQL("eql_v3_boolean")),
                }
            }
        });

        let statement = parse("SELECT DISTINCT * FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "DISTINCT * over a storage-only column should fail type checking"
        );
    }

    /// The schema the syntactic-form tests below share.
    ///
    /// `flag` is storage-only (`eql_v3_boolean` implements nothing), which is
    /// what the capability assertions use to check a form refuses a column that
    /// cannot support it.
    fn forms_schema() -> Arc<TableResolver> {
        resolver(schema! {
            tables: {
                t: {
                    id,
                    txt (EQL("eql_v3_text_search"): Eq + Ord + TokenMatch),
                    num (EQL("eql_v3_integer_ord"): Ord),
                    flag (EQL("eql_v3_boolean")),
                }
            }
        })
    }

    /// Every encrypted literal of `typed`, replaced by `'<CT>'`.
    fn dummy_encrypted_literals<'ast>(
        typed: &TypeCheckedStatement<'ast>,
    ) -> HashMap<NodeKey<'ast>, ast::Value> {
        typed
            .literals
            .iter()
            .map(|(_, v)| {
                (
                    NodeKey::new(*v),
                    ast::Value::SingleQuotedString("<CT>".to_string()),
                )
            })
            .collect()
    }

    /// Transforms `sql` with every encrypted literal replaced by `'<CT>'`.
    fn transform_with_dummy_literals(schema: Arc<TableResolver>, sql: &str) -> String {
        let statement = parse(sql);
        let typed = type_check(schema, &statement).unwrap();
        let encrypted = dummy_encrypted_literals(&typed);

        typed.transform(encrypted).unwrap().to_string()
    }

    /// Predicates that reduce to EQL's own operator overloads need no rewrite.
    ///
    /// EQL v3 ships `=`, `<`, `<=`, `>`, `>=` for every encrypted domain, each
    /// comparing the relevant term (`eql_v3.eq` is `eq_term(a) = eq_term(b)`).
    /// `IN` desugars to `= ANY(…)`, `BETWEEN` to `>= AND <=`, and
    /// `IS DISTINCT FROM` to a NULL-safe `=`, so all three are correct with the
    /// literals merely substituted.
    ///
    /// Pinning the pass-through matters because it looks wrong: the emitted SQL
    /// compares against a payload carrying the randomised ciphertext, and only
    /// operator resolution — invisible here — makes it right. The end-to-end
    /// rows are asserted in the integration suite.
    #[test]
    fn operator_backed_predicates_substitute_literals_without_wrapping() {
        let schema = forms_schema();

        for (input, expected) in [
            (
                "SELECT id FROM t WHERE txt IN ('a', 'b')",
                "SELECT id FROM t WHERE txt IN ('<CT>', '<CT>')",
            ),
            (
                "SELECT id FROM t WHERE txt NOT IN ('a', 'b')",
                "SELECT id FROM t WHERE txt NOT IN ('<CT>', '<CT>')",
            ),
            (
                "SELECT id FROM t WHERE num BETWEEN 1 AND 2",
                "SELECT id FROM t WHERE num BETWEEN '<CT>' AND '<CT>'",
            ),
            (
                "SELECT id FROM t WHERE txt IS DISTINCT FROM 'a'",
                "SELECT id FROM t WHERE txt IS DISTINCT FROM '<CT>'",
            ),
        ] {
            assert_eq!(
                transform_with_dummy_literals(schema.clone(), input),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// `BETWEEN` is ordering and `IS DISTINCT FROM` is equality, so each refuses
    /// a column whose domain carries no such term.
    #[test]
    fn operator_backed_predicates_require_their_capability() {
        let schema = forms_schema();

        for (input, bound) in [
            ("SELECT id FROM t WHERE flag BETWEEN true AND false", "Ord"),
            ("SELECT id FROM t WHERE flag IS DISTINCT FROM true", "Eq"),
        ] {
            let statement = parse(input);
            let err = type_check(schema.clone(), &statement)
                .expect_err(&format!("`{input}` should fail the capability check"));

            assert!(
                err.to_string().contains(bound),
                "expected a `{bound}` bound error for `{input}`, got: {err}"
            );
        }
    }

    /// `DISTINCT ON (col)` deduplicates, so it requires equality — and does
    /// enforce it, unlike the shapes below.
    #[test]
    fn distinct_on_requires_equality() {
        let schema = forms_schema();

        let statement = parse("SELECT DISTINCT ON (flag) flag FROM t");
        let err = type_check(schema, &statement)
            .expect_err("DISTINCT ON a storage-only column should fail the capability check");

        assert!(err.to_string().contains("Eq"), "unexpected error: {err}");
    }

    /// `IN` is equality, so it should refuse a column with no equality term.
    ///
    /// It does not: the `InList` arm of inference unifies the list against the
    /// column's type without a bound, so the shape type-checks and reaches the
    /// database.
    ///
    /// EQL is right to refuse it there — `eql_v3_boolean` is storage-only and
    /// `=` is deliberately unsupported on it, so the query fails with
    /// `operator = is not supported for public.eql_v3_boolean`. The gap is
    /// where the refusal comes from: `=` on the same column is caught by the
    /// capability check, `IN` is not, so the same mistake produces two very
    /// different errors depending on how it was written.
    #[test]
    fn in_list_requires_equality() {
        let schema = forms_schema();

        let statement = parse("SELECT id FROM t WHERE flag IN (true)");
        let err = type_check(schema, &statement)
            .expect_err("IN on a storage-only column should fail the capability check");

        assert!(err.to_string().contains("Eq"), "unexpected error: {err}");
    }

    /// One placeholder bound as both a stored value and a query operand.
    ///
    /// The two occurrences need different payloads — the stored one carries the
    /// ciphertext, the query one only search terms — so the role is per
    /// occurrence, not per input param. The rewritten SQL is the authority: the
    /// `SET` operand casts to the column's own domain, the `WHERE` operand to
    /// the `query_*` twin.
    #[test]
    fn param_reused_for_storage_and_query_keeps_separate_roles() {
        let schema = forms_schema();

        let statement = parse("UPDATE t SET txt = $1 WHERE txt = $1");
        let typed = type_check(schema, &statement).unwrap();
        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.statement.to_string(),
            "UPDATE t SET txt = $1::JSONB::public.eql_v3_text_search \
             WHERE eql_v3.eq_term(txt) = eql_v3.eq_term($2::JSONB::eql_v3.query_text_search)"
        );

        // Both outputs come from input $1, but only the WHERE one is a query
        // operand — marking both would strip the ciphertext from the stored
        // value and fail the column domain's CHECK.
        let roles: Vec<bool> = transformed
            .params
            .outputs()
            .iter()
            .map(|output| output.query_operand)
            .collect();

        assert_eq!(vec![false, true], roles);
    }

    /// A column that can be both traversed and compared for JSON equality.
    fn chained_json_schema() -> Arc<TableResolver> {
        resolver(schema! {
            tables: {
                t: {
                    id,
                    j (EQL("eql_v3_json_search"): Eq + Ord + JsonLike + Contain),
                }
            }
        })
    }

    /// A chained JSON accessor must not leave the intermediate selector in the
    /// statement, nor apply native `->` to the encrypted payload.
    ///
    /// The chain collapses into a single containment against the ROOT column:
    /// `j -> 'nested' -> 'string'` is the path `$.nested.string` of one
    /// document, and the needle is keyed on that whole path. Keeping the inner
    /// accessor (the container was cloned from the original AST) shipped the
    /// plaintext field name in the SQL text AND ran native jsonb `->` over an
    /// encrypted payload, so the predicate matched nothing either.
    #[test]
    fn chained_json_accessor_does_not_emit_the_plaintext_selector() {
        let rewritten = transform_with_dummy_literals(
            chained_json_schema(),
            "SELECT id FROM t WHERE j -> 'nested' -> 'string' = '\"world\"'",
        );

        assert_eq!(
            rewritten,
            "SELECT id FROM t WHERE eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json)"
        );

        assert!(
            !rewritten.contains("'nested'"),
            "the intermediate selector must not reach the database in plaintext: {rewritten}"
        );
        assert!(
            !rewritten.contains("j -> "),
            "native jsonb -> must not be applied to the encrypted column: {rewritten}"
        );
    }

    /// A JSON operation on the RESULT of a JSON operation must be refused when
    /// the two are not in the same expression.
    ///
    /// `->` yields `EqlTerm::JsonExtracted` — one SteVec entry, which carries no
    /// `sv` array and so cannot be traversed. A chain written in one expression
    /// is collapsed into a single path against the document instead, but once the
    /// halves are separated by a subquery the selectors cannot be composed: the
    /// walker sees only `a -> 'foo'` and has no way to learn that `a` is already
    /// `$.bar`. It used to emit a second entry-scoped accessor over an entry and
    /// return NULL, silently, or fuse a needle keyed on `$.foo` when the real
    /// path was `$.bar.foo` — wrong rows, no error (CIP-3682).
    ///
    /// This one is impossible rather than unimplemented, and stays refused even
    /// though a chain in one expression now works everywhere: `JsonExtracted` does
    /// not carry the path that produced it, and the root column is not in scope in
    /// the outer query, so there is nothing to root a composed path at.
    ///
    /// A type crosses a subquery boundary where a syntactic pattern does not,
    /// which is why this is carried in the type system rather than by the walker.
    #[test]
    fn json_operation_on_an_extracted_value_is_refused() {
        let schema = chained_json_schema();

        for sql in [
            // The reported shape: the chain split across a subquery.
            "SELECT a -> 'foo' FROM (SELECT j -> 'bar' AS a FROM t) s",
            // The same split, but where the outer half is a fusable predicate.
            // The fusion must NOT claim this: its root is an entry, not a
            // document, so the path it would compose is wrong.
            "SELECT id FROM (SELECT j -> 'bar' AS a, id FROM t) s WHERE a -> 'foo' = '\"x\"'",
            // The `->>` spelling is the same operation.
            "SELECT a ->> 'foo' FROM (SELECT j -> 'bar' AS a FROM t) s",
        ] {
            let statement = parse(sql);
            let err = type_check(schema.clone(), &statement)
                .expect_err(&format!("`{sql}` must not type check"));

            assert!(
                err.to_string()
                    .contains("result of an encrypted JSON operation"),
                "expected an unqueryable-extraction error for `{sql}`, got: {err}"
            );
        }
    }

    /// A multi-step chain collapses to a SINGLE accessor on the root document,
    /// in every context — not only under an equality.
    ///
    /// A chain cannot be two hops: `eql_v3."->"` searches the document's `sv`
    /// array and returns one entry, which has no `sv` of its own, so an accessor
    /// over an accessor finds nothing and returns NULL. The emission that IS
    /// correct is one accessor carrying the composed path, and it is correct
    /// wherever a single access is — so a projection, an ordering comparison and a
    /// mixed spelling all produce exactly the shape the equivalent single access
    /// would.
    ///
    /// The plaintext selectors of the discarded inner accessors must go with them:
    /// leaving one behind ships a field name to PostgreSQL in the clear
    /// (CIP-3682) and applies native jsonb `->` to an encrypted payload.
    #[test]
    fn a_multi_step_chain_collapses_to_one_accessor_in_every_context() {
        let schema = chained_json_schema();

        // Each case pairs a chain with the emission expected of it. The selector
        // is `'<CT>'` in every one: the whole path is keyed into that single
        // encrypted operand, so the SQL cannot show how many steps there were.
        for (sql, expected) in [
            // A projection, which has no comparison to fuse into.
            (
                "SELECT j -> 'foo' -> 'bar' FROM t",
                "SELECT eql_v3.\"->\"(j, '<CT>') FROM t",
            ),
            // Brackets are not meaning: the same query, the same emission.
            (
                "SELECT (j -> 'foo') -> 'bar' FROM t",
                "SELECT eql_v3.\"->\"(j, '<CT>') FROM t",
            ),
            // Mixed spellings. The OUTERMOST step decides the call, exactly as it
            // would for a single access: `->>` yields text.
            (
                "SELECT j -> 'foo' ->> 'bar' FROM t",
                "SELECT eql_v3.\"->>\"(j, '<CT>') FROM t",
            ),
            // Depth beyond two.
            (
                "SELECT j -> 'a' -> 'b' -> 'c' FROM t",
                "SELECT eql_v3.\"->\"(j, '<CT>') FROM t",
            ),
            // The function spelling as a step of the chain.
            (
                "SELECT jsonb_path_query_first(j, '$.a') -> 'b' FROM t",
                "SELECT eql_v3.\"->\"(j, '<CT>') FROM t",
            ),
            // Ordering. The accessor survives here — the comparison wraps it in
            // `ord_term` rather than absorbing it — so it must be the collapsed
            // single-accessor form.
            (
                "SELECT id FROM t WHERE j -> 'foo' -> 'bar' < '\"x\"'",
                "SELECT id FROM t WHERE eql_v3.ord_term(eql_v3.\"->\"(j, '<CT>')) < \
                 eql_v3.ord_term('<CT>'::JSONB::eql_v3.query_integer_ord)",
            ),
            (
                "SELECT id FROM t WHERE j -> 'foo' -> 'bar' >= '\"x\"'",
                "SELECT id FROM t WHERE eql_v3.ord_term(eql_v3.\"->\"(j, '<CT>')) >= \
                 eql_v3.ord_term('<CT>'::JSONB::eql_v3.query_integer_ord)",
            ),
        ] {
            let rewritten = transform_with_dummy_literals(schema.clone(), sql);

            assert_eq!(rewritten, expected, "unexpected rewrite for `{sql}`");

            for selector in ["'foo'", "'bar'", "'a'", "'b'", "'$.a'"] {
                assert!(
                    !rewritten.contains(selector),
                    "selector {selector} must not reach the database in plaintext for `{sql}`: {rewritten}"
                );
            }
            assert!(
                !rewritten.contains("j -> "),
                "native jsonb -> must not be applied to the encrypted column for `{sql}`: {rewritten}"
            );
        }
    }

    /// A chain collapsed outside an equality records its composed path against
    /// the SURVIVING selector operand, which is the outermost step.
    ///
    /// That operand's own text is one segment (`'c'`); the selector it must key is
    /// the whole path (`$.a.<$1>.c`). Nothing the proxy is handed at encryption
    /// time could recover the difference, so the record is the only way the inner
    /// steps reach the needle — and every step is an input the plan must consume,
    /// or the client would bind a param that goes nowhere.
    #[test]
    fn a_collapsed_chain_records_its_composed_path_against_the_surviving_selector() {
        // The outermost step is the param, so the whole path resolves at Bind.
        let statement = parse("SELECT j -> 'a' -> $1 FROM t");

        let typed = type_check(chained_json_schema(), &statement).unwrap();
        let transformed = typed.transform(dummy_encrypted_literals(&typed)).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT eql_v3.\"->\"(j, $1) FROM t"
        );

        let source = OutputParamSource::JsonAccessorPath {
            path: JsonSelectorSource::new(vec![
                JsonSelectorSegment::Literal("a".to_owned()),
                JsonSelectorSegment::Param(Param(1)),
            ]),
            selector: Param(1),
        };

        assert_eq!(transformed.params.outputs()[0].source, source);
        assert_eq!(source.inputs(), vec![Param(1)]);

        // The selector is a query operand: it reaches PostgreSQL as a search term
        // and never as a decryptable ciphertext.
        assert!(transformed.params.outputs()[0].query_operand);
    }

    /// An EQUALITY over a chain must keep fusing, not degrade to an accessor plus
    /// a comparison.
    ///
    /// The fused needle keys the path and the value together into one MAC, and its
    /// presence in the stored `sv` IS the match. An accessor followed by `eq_term`
    /// would be two operations where one suffices, and `eql_v3.eq_term` has no
    /// overload for a JSON query operand anyway. The chain-collapsing rule fires
    /// on the accessor below the comparison, and the equality rule then discards
    /// its result and re-roots the containment at the bare column.
    #[test]
    fn equality_over_a_chain_still_fuses_rather_than_collapsing_to_an_accessor() {
        let schema = chained_json_schema();

        for (sql, expected) in [
            (
                "SELECT id FROM t WHERE j -> 'a' -> 'b' = '\"v\"'",
                "SELECT id FROM t WHERE \
                 eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json)",
            ),
            (
                "SELECT id FROM t WHERE j -> 'a' -> 'b' <> '\"v\"'",
                "SELECT id FROM t WHERE \
                 NOT (eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json))",
            ),
        ] {
            let rewritten = transform_with_dummy_literals(schema.clone(), sql);

            assert_eq!(rewritten, expected, "unexpected rewrite for `{sql}`");
            assert!(
                !rewritten.contains("eql_v3.\"->\""),
                "equality must fuse, not emit an accessor, for `{sql}`: {rewritten}"
            );
        }
    }

    /// A fused equality records its path in the value-selector channel ONLY.
    ///
    /// Recording it in both would be worse than recording it in neither: the
    /// accessor channel is resolved at Parse time for a literal operand, and
    /// `j -> $1 -> 'b' = $2` has a placeholder step in front of a literal
    /// selector, which cannot resolve then. Writing the path to both channels
    /// would refuse a query that works.
    #[test]
    fn a_fused_chain_records_no_accessor_path() {
        let statement = parse("SELECT id FROM t WHERE j -> 'a' -> 'b' = $1");
        let typed = type_check(chained_json_schema(), &statement).unwrap();

        assert!(
            typed.json_accessor_paths.is_empty(),
            "a chain the equality absorbs has no surviving selector to key"
        );
        assert!(!typed.json_value_selectors.is_empty());
    }

    /// Every step of a collapsed chain must be resolvable to path text.
    ///
    /// The chain is collapsed either way, so a step the proxy cannot resolve — a
    /// column reference, a function call — would simply vanish from the statement
    /// and the query would read a different field. Unlike the fused-equality case
    /// there is no capability check to fall through to, so it is refused outright.
    #[test]
    fn a_collapsed_chain_with_an_unresolvable_step_is_refused() {
        let statement = parse("SELECT j -> 'a' -> id FROM t");

        let err = type_check(chained_json_schema(), &statement)
            .expect_err("a path step that is not a literal or a placeholder must be refused");

        assert!(
            err.to_string()
                .contains("must be a literal or a placeholder"),
            "expected an uncomposable-path error, got: {err}"
        );
    }

    /// One placeholder cannot be the selector of two chains with different paths.
    ///
    /// The path is recorded against the param it arrives in, because at Bind time
    /// the param number is all the proxy has. Two different paths for one param
    /// cannot both be honoured, and silently keeping either would answer one of
    /// the two projections from the wrong field.
    #[test]
    fn one_placeholder_cannot_key_two_different_paths() {
        let statement = parse("SELECT j -> 'a' -> $1, j -> 'b' -> $1 FROM t");

        let err = type_check(chained_json_schema(), &statement)
            .expect_err("one param cannot carry two different paths");

        assert!(
            err.to_string().contains("two different"),
            "expected an ambiguous-path error, got: {err}"
        );

        // The same path twice is not a conflict — it is one path.
        let statement = parse("SELECT j -> 'a' -> $1, j -> 'a' -> $1 FROM t");
        type_check(chained_json_schema(), &statement).unwrap();
    }

    /// Sorting by an extracted JSON field sorts by its ordering term.
    ///
    /// An extracted SteVec entry carries ordering and equality terms, so
    /// `ORDER BY col -> 'field'` is legitimate — it sorts by
    /// `ord_term(eql_v3."->"(col, sel))`. This pins the capability grant on
    /// `EqlTerm::JsonExtracted`: when it briefly had NO capabilities at all,
    /// this exact shape failed the `Ord` bound, and with mapping errors
    /// disabled (the container default) the statement was forwarded unmapped —
    /// native `->` over ciphertext, zero rows, silently. The showcase's
    /// "active Aspirin prescriptions" query was the first thing to notice.
    #[test]
    fn order_by_an_extracted_json_field_sorts_by_its_ordering_term() {
        let rewritten = transform_with_dummy_literals(
            chained_json_schema(),
            "SELECT id FROM t ORDER BY j -> 'email'",
        );

        assert_eq!(
            rewritten,
            "SELECT id FROM t ORDER BY eql_v3.ord_term(eql_v3.\"->\"(j, '<CT>'))"
        );
    }

    /// A SINGLE access is legal anywhere, fused or not.
    ///
    /// The declaration handles it: `-> <T as JsonLike>::Output` yields an
    /// extracted entry, which is projectable and decryptable. Only *traversing*
    /// that result is refused, so ordinary field access and single-field
    /// comparisons are unaffected.
    #[test]
    fn a_single_json_access_is_legal_anywhere() {
        let schema = chained_json_schema();

        for sql in [
            "SELECT j -> 'foo' FROM t",
            "SELECT j ->> 'foo' FROM t",
            "SELECT id FROM t WHERE j -> 'foo' = '\"x\"'",
            "SELECT id FROM t WHERE j -> 'foo' < '\"x\"'",
        ] {
            let statement = parse(sql);
            type_check(schema.clone(), &statement)
                .unwrap_or_else(|e| panic!("`{sql}` should type check, got: {e}"));
        }
    }

    /// Extracting one field, and projecting an extracted field, both still work.
    ///
    /// The point of `JsonExtracted` is to forbid *traversing* an extracted entry,
    /// not to make extraction useless: a single access is the common case, and
    /// the result is projectable and decryptable exactly as before.
    #[test]
    fn extracting_and_projecting_one_json_field_still_works() {
        let schema = chained_json_schema();

        assert_eq!(
            transform_with_dummy_literals(schema.clone(), "SELECT j -> 'foo' FROM t"),
            "SELECT eql_v3.\"->\"(j, '<CT>') FROM t"
        );

        // An extracted entry crossing a subquery boundary is fine as long as
        // nothing traverses it on the far side.
        assert_eq!(
            transform_with_dummy_literals(
                schema,
                "SELECT a FROM (SELECT j -> 'bar' AS a FROM t) s"
            ),
            "SELECT a FROM (SELECT eql_v3.\"->\"(j, '<CT>') AS a FROM t) AS s"
        );
    }

    /// Parentheses must not defeat the chain walker.
    ///
    /// `(j -> 'foo') -> 'bar'` is `j -> 'foo' -> 'bar'` with redundant brackets,
    /// and must fuse to the same needle rooted at the same bare column. Before
    /// the walker saw through `Expr::Nested` it stopped at the bracket, treated
    /// the parenthesised accessor as the ROOT container, and emitted
    /// `eql_v3.jsonb_contains((j -> 'foo'), …)` — shipping the plaintext selector
    /// `'foo'` to PostgreSQL and applying native jsonb `->` to the encrypted
    /// payload. The same CIP-3682 leak, reachable with one pair of brackets.
    #[test]
    fn parenthesised_json_accessor_chains_fuse_identically() {
        let schema = chained_json_schema();

        // Every spelling below is the same query, so every one must produce the
        // same fused containment against the bare root column.
        let expected =
            "SELECT id FROM t WHERE eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json)";

        for sql in [
            "SELECT id FROM t WHERE j -> 'foo' -> 'bar' = '\"x\"'",
            "SELECT id FROM t WHERE (j -> 'foo') -> 'bar' = '\"x\"'",
            "SELECT id FROM t WHERE ((j -> 'foo') -> 'bar') = '\"x\"'",
            "SELECT id FROM t WHERE (((j -> 'foo')) -> 'bar') = '\"x\"'",
            "SELECT id FROM t WHERE (j) -> 'foo' -> 'bar' = '\"x\"'",
            "SELECT id FROM t WHERE j -> ('foo') -> 'bar' = '\"x\"'",
        ] {
            let rewritten = transform_with_dummy_literals(schema.clone(), sql);

            assert_eq!(rewritten, expected, "unexpected rewrite for `{sql}`");

            assert!(
                !rewritten.contains("'foo'"),
                "the intermediate selector must not reach the database in plaintext for `{sql}`: {rewritten}"
            );
            assert!(
                !rewritten.contains("j -> "),
                "native jsonb -> must not be applied to the encrypted column for `{sql}`: {rewritten}"
            );
        }
    }

    /// Every spelling and depth of chain collapses the same way, and none of
    /// them leaves a selector behind.
    #[test]
    fn chained_json_accessor_spellings_all_collapse_to_root_containment() {
        let cases = [
            // Depth 3, and deeper.
            "j -> 'a' -> 'b' -> 'c' = '\"v\"'",
            "j -> 'a' -> 'b' -> 'c' -> 'd' = '\"v\"'",
            // The `->>` spelling, and mixed with `->`.
            "j ->> 'a' = '\"v\"'",
            "j -> 'a' ->> 'b' = '\"v\"'",
            "j ->> 'a' ->> 'b' = '\"v\"'",
            // The function spelling, rooted and chained.
            "jsonb_path_query_first(j, '$.a') = '\"v\"'",
            "jsonb_path_query_first(j, '$.a') -> 'b' = '\"v\"'",
            // The value operand written on the left.
            "'\"v\"' = j -> 'a' -> 'b'",
        ];

        for case in cases {
            let rewritten = transform_with_dummy_literals(
                chained_json_schema(),
                &format!("SELECT id FROM t WHERE {case}"),
            );

            assert_eq!(
                rewritten,
                "SELECT id FROM t WHERE eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json)",
                "unexpected rewrite for `{case}`"
            );
        }
    }

    /// `<>` on a chain is the same containment, negated — the selectors are
    /// discarded there too.
    #[test]
    fn chained_json_accessor_not_eq_rewrites_to_negated_containment() {
        let rewritten = transform_with_dummy_literals(
            chained_json_schema(),
            "SELECT id FROM t WHERE j -> 'nested' -> 'string' <> '\"world\"'",
        );

        assert_eq!(
            rewritten,
            "SELECT id FROM t WHERE NOT (eql_v3.jsonb_contains(j, '<CT>'::JSONB::eql_v3.query_json))"
        );
    }

    /// The path a chain composes is recorded step by step, so the proxy can
    /// build `$.a.<$1>.c` once the placeholder steps are bound. Every step is an
    /// input the plan must consume — dropping one would leave the client binding
    /// a param that never reaches the needle.
    #[test]
    fn chained_json_accessor_records_every_path_step() {
        let statement = parse("SELECT id FROM t WHERE j -> 'a' -> $1 -> 'c' = $2");

        let typed = type_check(chained_json_schema(), &statement).unwrap();
        let transformed = typed.transform(dummy_encrypted_literals(&typed)).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM t WHERE eql_v3.jsonb_contains(j, $1::JSONB::eql_v3.query_json)"
        );

        let source = OutputParamSource::JsonValueSelector {
            path: JsonSelectorSource::new(vec![
                JsonSelectorSegment::Literal("a".to_owned()),
                JsonSelectorSegment::Param(Param(1)),
                JsonSelectorSegment::Literal("c".to_owned()),
            ]),
            value: Param(2),
        };

        assert_eq!(transformed.params.outputs()[0].source, source);
        assert_eq!(source.inputs(), vec![Param(1), Param(2)]);
    }

    /// A chain with a step that is neither a literal nor a placeholder cannot
    /// be composed into a path, so the fusion is declined and the comparison
    /// falls through to the ordinary capability check — an error, not a
    /// half-built needle and not a leak.
    #[test]
    fn chained_json_accessor_with_an_unresolvable_step_is_rejected() {
        let statement = parse("SELECT id FROM t WHERE j -> 'a' -> id = '\"v\"'");

        assert!(
            type_check(chained_json_schema(), &statement).is_err(),
            "a path step that is not a literal or a placeholder must not fuse"
        );
    }

    /// JSON field access requires the column to support field selection.
    #[test]
    fn json_field_access_requires_json_like() {
        let schema = forms_schema();

        let statement = parse("SELECT id FROM t WHERE txt -> 'a' = '\"b\"'");

        assert!(
            type_check(schema, &statement).is_err(),
            "field access on a non-JSON encrypted column should fail the capability check"
        );
    }

    /// A simple `CASE` compares its operand for equality and returns its
    /// results — two independent types.
    ///
    /// Conflating them typed the results as the operand: the integer arms of
    /// `CASE enc WHEN 'a' THEN 1 ELSE 0 END` were encrypted as values of the
    /// encrypted column and shipped as EQL payloads where plain integers belong.
    #[test]
    fn simple_case_keeps_its_result_type_independent_of_the_operand() {
        let schema = forms_schema();

        assert_eq!(
            transform_with_dummy_literals(
                schema.clone(),
                "SELECT CASE txt WHEN 'a' THEN 1 ELSE 0 END AS c FROM t",
            ),
            "SELECT CASE txt WHEN '<CT>' THEN 1 ELSE 0 END AS c FROM t",
        );

        // The searched form was always correct; it stays that way.
        assert_eq!(
            transform_with_dummy_literals(
                schema,
                "SELECT CASE WHEN txt = 'a' THEN 1 ELSE 0 END AS c FROM t",
            ),
            "SELECT CASE WHEN eql_v3.eq_term(txt) = \
             eql_v3.eq_term('<CT>'::JSONB::eql_v3.query_text_search) THEN 1 ELSE 0 END AS c FROM t",
        );
    }

    /// The operand is compared for equality, so its domain must carry an
    /// equality term.
    #[test]
    fn simple_case_operand_requires_equality() {
        let schema = forms_schema();

        let statement = parse("SELECT CASE flag WHEN true THEN 1 ELSE 0 END AS c FROM t");
        let err = type_check(schema, &statement)
            .expect_err("CASE on a storage-only operand should fail the capability check");

        assert!(err.to_string().contains("Eq"), "unexpected error: {err}");
    }

    /// A set operation that deduplicates cannot do so on an encrypted column.
    ///
    /// Deduplication goes through the type's default operator class rather than
    /// EQL's `=` overload, so it compares whole payloads including the
    /// randomised ciphertext — `UNION` keeps every duplicate. Unlike
    /// `SELECT DISTINCT` it cannot be keyed on the equality term in place,
    /// because deduplication spans the whole projection of both branches, so it
    /// is refused rather than silently wrong.
    #[test]
    fn deduplicating_set_operations_on_encrypted_columns_are_rejected() {
        let schema = forms_schema();

        for input in [
            "SELECT txt FROM t UNION SELECT txt FROM t",
            "SELECT txt FROM t INTERSECT SELECT txt FROM t",
            "SELECT txt FROM t EXCEPT SELECT txt FROM t",
        ] {
            let statement = parse(input);
            let err = type_check(schema.clone(), &statement)
                .expect_err(&format!("`{input}` should be refused"));

            assert!(
                err.to_string()
                    .contains("deduplication would compare ciphertexts"),
                "unexpected error for `{input}`: {err}"
            );
        }

        // `ALL` performs no deduplication, so it is unaffected.
        for input in [
            "SELECT txt FROM t UNION ALL SELECT txt FROM t",
            "SELECT id FROM t UNION SELECT id FROM t",
        ] {
            let statement = parse(input);
            assert!(
                type_check(schema.clone(), &statement).is_ok(),
                "`{input}` should be accepted"
            );
        }
    }

    /// `DISTINCT ON (col)` deduplicates, so each encrypted key is keyed on its
    /// equality term — in place, since the keys are named explicitly.
    #[test]
    fn distinct_on_keys_on_the_equality_term() {
        let schema = forms_schema();

        for (input, expected) in [
            (
                "SELECT DISTINCT ON (txt) txt FROM t",
                "SELECT DISTINCT ON (eql_v3.eq_term(txt)) txt FROM t",
            ),
            // A plaintext key is left alone.
            (
                "SELECT DISTINCT ON (id, txt) txt FROM t",
                "SELECT DISTINCT ON (id, eql_v3.eq_term(txt)) txt FROM t",
            ),
            (
                "SELECT DISTINCT ON (id) id FROM t",
                "SELECT DISTINCT ON (id) id FROM t",
            ),
        ] {
            assert_eq!(
                transform_with_dummy_literals(schema.clone(), input),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// `ORDER BY <ordinal>` and `GROUP BY <ordinal>` selecting an encrypted
    /// column are rewritten to that column's term.
    ///
    /// An ordinal names no column of its own, so the rules that match on the
    /// key's type saw only a number and left the clause alone — sorting and
    /// grouping then fell back to jsonb over the randomised ciphertext.
    /// PostgreSQL defines `ORDER BY n` as ordering by the n-th output column, so
    /// substituting that column is semantics-preserving.
    #[test]
    fn ordinal_sort_and_group_keys_use_the_columns_term() {
        let schema = forms_schema();

        for (input, expected) in [
            (
                "SELECT txt FROM t ORDER BY 1",
                "SELECT txt FROM t ORDER BY eql_v3.ord_term(txt)",
            ),
            // Sort options ride along, and the ordinal may be any position.
            (
                "SELECT id, txt FROM t ORDER BY 2 DESC",
                "SELECT id, txt FROM t ORDER BY eql_v3.ord_term(txt) DESC",
            ),
            // Grouping keys on the equality term, and the projected column is
            // lifted through `grouped_value` exactly as for a named key.
            (
                "SELECT txt FROM t GROUP BY 1",
                "SELECT eql_v3.grouped_value(txt) AS txt FROM t GROUP BY eql_v3.eq_term(txt)",
            ),
            // An ordinal selecting a plaintext column is left alone.
            ("SELECT id FROM t ORDER BY 1", "SELECT id FROM t ORDER BY 1"),
            ("SELECT id FROM t GROUP BY 1", "SELECT id FROM t GROUP BY 1"),
        ] {
            assert_eq!(
                transform_with_dummy_literals(schema.clone(), input),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// An ordinal key carries the same capability requirement as a named one.
    #[test]
    fn ordinal_sort_and_group_keys_require_their_capability() {
        let schema = forms_schema();

        for (input, bound) in [
            ("SELECT flag FROM t ORDER BY 1", "Ord"),
            ("SELECT flag FROM t GROUP BY 1", "Eq"),
        ] {
            let statement = parse(input);
            let err = type_check(schema.clone(), &statement)
                .expect_err(&format!("`{input}` should fail the capability check"));

            assert!(
                err.to_string().contains(bound),
                "expected a `{bound}` bound error for `{input}`, got: {err}"
            );
        }
    }

    /// `PARTITION BY` groups rows by equality, so an encrypted key is
    /// partitioned on its equality term.
    ///
    /// The window's own `ORDER BY` is handled by `RewriteEqlOrderBy`, which
    /// matches `OrderByExpr` wherever it appears — including inside a window.
    #[test]
    fn window_partition_by_uses_the_columns_term() {
        let schema = forms_schema();

        for (input, expected) in [
            (
                "SELECT rank() OVER (PARTITION BY txt) FROM t",
                "SELECT rank() OVER (PARTITION BY eql_v3.eq_term(txt)) FROM t",
            ),
            (
                "SELECT rank() OVER (PARTITION BY txt ORDER BY num) FROM t",
                "SELECT rank() OVER (PARTITION BY eql_v3.eq_term(txt) \
                 ORDER BY eql_v3.ord_term(num)) FROM t",
            ),
            // A plaintext partition key is left alone.
            (
                "SELECT rank() OVER (PARTITION BY id ORDER BY txt) FROM t",
                "SELECT rank() OVER (PARTITION BY id ORDER BY eql_v3.ord_term(txt)) FROM t",
            ),
        ] {
            assert_eq!(
                transform_with_dummy_literals(schema.clone(), input),
                expected.split_whitespace().collect::<Vec<_>>().join(" "),
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// Partitioning is equality, so the key's domain must carry an equality
    /// term.
    #[test]
    fn window_partition_by_requires_equality() {
        let schema = forms_schema();

        let statement = parse("SELECT rank() OVER (PARTITION BY flag) FROM t");
        let err = type_check(schema, &statement)
            .expect_err("PARTITION BY a storage-only column should fail the capability check");

        assert!(err.to_string().contains("Eq"), "unexpected error: {err}");
    }

    /// Shapes the subquery rewrite cannot express are reported as such, rather
    /// than left to fail as a bare PostgreSQL syntax error.
    #[test]
    fn distinct_order_by_rejects_inexpressible_shapes() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected_fragment) in [
            // DISTINCT ON constrains ORDER BY to begin with its own expressions;
            // wrapping would silently break that.
            (
                "SELECT DISTINCT ON (id) id, salary FROM employees ORDER BY id, salary",
                "DISTINCT ON",
            ),
            // A wildcard cannot be named, so the outer projection cannot
            // reproduce it column for column.
            (
                "SELECT DISTINCT * FROM employees ORDER BY salary",
                "wildcard",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            let err = typed
                .transform(HashMap::new())
                .expect_err(&format!("expected `{input}` to be rejected"));

            assert!(
                err.to_string().contains(expected_fragment),
                "unexpected error for `{input}`: {err}"
            );
        }
    }

    /// `GROUP BY` on an encrypted column groups by its equality term. A bare
    /// `GROUP BY col` groups on the jsonb payload, whose ciphertext is
    /// randomised per row, so equal plaintexts land in different groups.
    ///
    /// Projecting the grouped column has to be lifted through
    /// `eql_v3.grouped_value`, because PostgreSQL no longer sees it as
    /// functionally dependent on the group key — and the projection keeps the
    /// name the client asked for.
    #[test]
    fn group_by_encrypted_column_uses_eq_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    email (EQL("eql_v3_text_search"): Eq + Ord + TokenMatch),
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        for (input, expected) in [
            // The column is not projected: only the group key changes.
            (
                "SELECT COUNT(*) FROM employees GROUP BY email",
                "SELECT COUNT(*) FROM employees GROUP BY eql_v3.eq_term(email)",
            ),
            // Projected: lifted through `grouped_value`, keeping its name.
            (
                "SELECT email FROM employees GROUP BY email",
                "SELECT eql_v3.grouped_value(email) AS email FROM employees GROUP BY eql_v3.eq_term(email)",
            ),
            // An explicit alias is preserved as-is.
            (
                "SELECT email AS e FROM employees GROUP BY email",
                "SELECT eql_v3.grouped_value(email) AS e FROM employees GROUP BY eql_v3.eq_term(email)",
            ),
            // Qualified projection of the same column still matches — the match
            // is on the resolved column, not on syntax.
            (
                "SELECT employees.email FROM employees GROUP BY email",
                "SELECT eql_v3.grouped_value(employees.email) AS email FROM employees GROUP BY eql_v3.eq_term(email)",
            ),
            // A domain that stores no `hm` groups by its ordering term, the same
            // fallback `=` uses.
            (
                "SELECT COUNT(*) FROM employees GROUP BY salary",
                "SELECT COUNT(*) FROM employees GROUP BY eql_v3.ord_term(salary)",
            ),
            // A native group key is left alone.
            (
                "SELECT COUNT(*) FROM employees GROUP BY id",
                "SELECT COUNT(*) FROM employees GROUP BY id",
            ),
        ] {
            let statement = parse(input);
            let typed = type_check(schema.clone(), &statement).unwrap();

            assert_eq!(
                typed.transform(HashMap::new()).unwrap().to_string(),
                expected,
                "unexpected rewrite for `{input}`"
            );
        }
    }

    /// Grouping by a column whose domain carries no equality term is a
    /// capability error, not a grouping on ciphertext.
    #[test]
    fn group_by_column_without_equality_term_is_an_error() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name (EQL("eql_v3_text_match"): TokenMatch),
                }
            }
        });

        // Caught by the `Eq` bound during type checking rather than by the
        // rewrite: the clause is typed now, so the capability is checked before
        // any SQL is produced.
        let statement = parse("SELECT COUNT(*) FROM employees GROUP BY name");
        let err = type_check(schema, &statement)
            .expect_err("GROUP BY on a match-only column should fail the capability check")
            .to_string();

        assert!(
            err.contains("Eq"),
            "expected an `Eq` bound error, got: {err}"
        );
    }

    /// A block-ORE domain orders through `ord_term_ore`.
    #[test]
    fn order_by_ore_column_uses_ord_term_ore() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord_ore"): Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM employees ORDER BY salary");
        let typed = type_check(schema, &statement).unwrap();

        assert_eq!(
            typed.transform(HashMap::new()).unwrap().to_string(),
            "SELECT id FROM employees ORDER BY eql_v3.ord_term_ore(salary)"
        );
    }

    /// Ordering by a column whose domain carries no ordering term is a
    /// capability error, not an arbitrary sort.
    #[test]
    fn order_by_column_without_ordering_term_is_an_error() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    name (EQL("eql_v3_text_match"): TokenMatch),
                }
            }
        });

        // Caught by the `Ord` bound during type checking rather than by the
        // rewrite, as for GROUP BY above.
        let statement = parse("SELECT id FROM employees ORDER BY name");
        let err = type_check(schema, &statement)
            .expect_err("ORDER BY on a match-only column should fail the capability check")
            .to_string();

        assert!(
            err.contains("Ord"),
            "expected an `Ord` bound error, got: {err}"
        );
    }

    #[test]
    fn jsonb_path_query_param_to_eql() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike),
                }
            }
        });

        let statement = parse("SELECT eql_v3.jsonb_path_query(notes, $1) as notes FROM patients");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // A path query still yields the column's own type, NOT `JsonExtracted`.
        //
        // `->`/`->>` return `<T as JsonLike>::Output` so a second traversal of an
        // encrypted result is refused. The path-query functions deliberately do
        // NOT, because two supported shapes depend on the old typing:
        // `jsonb_array_elements`/`jsonb_array_length` consume an extracted entry
        // rather than traversing it, and the rewrite that retargets these
        // functions and encrypts their Path operand keys off the result type —
        // changing it sent the caller's literal jsonpath to PostgreSQL
        // unencrypted. Closing that needs the array functions taught to accept
        // an extracted value first.
        assert_eq!(
            typed.projection,
            projection![(EQL(patients.notes: JsonLike) as notes)]
        );
    }

    #[test]
    fn ensure_eql_mapper_does_not_choke_on_elixir_ecto_schema_metadata_query() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                pg_attribute: {
                    attrelid,
                    attnum,
                    atttypid,
                    attisdropped,
                }
                pg_type: {
                    oid,
                    typname,
                    typsend,
                    typreceive,
                    typoutput,
                    typinput,
                    typbasetype,
                    typrelid,
                    typelem,
                }
                pg_range: {
                   rngtypid,
                   rngmultitypid,
                   rngsubtype,
                }
            }
        });

        let statement = parse(
            "SELECT
            t.oid,
            t.typname,
            t.typsend,
            t.typreceive,
            t.typoutput,
            t.typinput,
            coalesce(d.typelem, t.typelem),
            coalesce(r.rngsubtype, 0),
            ARRAY(
                SELECT
                    a.atttypid
                FROM
                    pg_attribute AS a
                WHERE
                    a.attrelid = t.typrelid
                    AND a.attnum > 0
                    AND NOT a.attisdropped
                ORDER BY a.attnum
            ) FROM pg_type AS t
                LEFT JOIN pg_type AS d ON t.typbasetype = d.oid
                LEFT JOIN pg_range AS r ON r.rngtypid = t.oid OR r.rngmultitypid = t.oid OR (
                    t.typbasetype <> 0
                    AND r.rngtypid = t.typbasetype
                )
            WHERE
                (t.typrelid = 0)
            AND (t.typelem = 0 OR NOT EXISTS (
                SELECT 1 FROM pg_type AS s
                WHERE s.typrelid <> 0 AND s.oid = t.typelem
            ))",
        );

        type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();
    }

    #[test]
    fn functions_can_be_resolved_case_insensitively() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    age (EQL: Ord),
                }
            }
        });

        let statement = parse(
            r#"
            select min(age), MIN(age) from patients;
        "#,
        );

        type_check(schema, &statement).unwrap();
    }

    // -----------------------------------------------------------------------
    // CIP-3699: clauses that previously escaped inference on statements that
    // pass `requires_type_check`. Each test either proves the new bound plus
    // rewrite, or proves the explicit rejection.
    // -----------------------------------------------------------------------

    /// `ON CONFLICT DO UPDATE SET enc = <literal>` is the upsert path: the
    /// assignment value must be typed as the column's EQL type so the literal
    /// is encrypted, exactly as in a plain `UPDATE ... SET`.
    #[test]
    fn insert_on_conflict_do_update_encrypts_assignment_literal() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            "INSERT INTO employees (id, salary) VALUES (1, 20000) \
             ON CONFLICT (id) DO UPDATE SET salary = 30000",
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        // Both the inserted literal AND the conflict-path literal are EQL
        // literals to encrypt.
        assert_eq!(typed.literals.len(), 2);

        let encrypted = typed
            .literals
            .iter()
            .map(|(_, node)| {
                (
                    node.as_node_key(),
                    ast::Value::SingleQuotedString(format!("ENCRYPTED_{node}")),
                )
            })
            .collect::<HashMap<_, _>>();

        match typed.transform(encrypted) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "INSERT INTO employees (id, salary) VALUES (1, 'ENCRYPTED_20000'::JSONB::public.eql_v3_text) \
                 ON CONFLICT(id) DO UPDATE SET salary = 'ENCRYPTED_30000'::JSONB::public.eql_v3_text"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `excluded` pseudo-table projects the target table's columns, so
    /// `SET enc = excluded.enc` resolves and the param feeding the insert
    /// value gets the column's EQL type.
    #[test]
    fn insert_on_conflict_do_update_with_excluded_reference() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            "INSERT INTO employees (id, salary) VALUES ($1, $2) \
             ON CONFLICT (id) DO UPDATE SET salary = excluded.salary",
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert!(
            matches!(
                &typed.params[..],
                [(_, Value::Native(_)), (_, Value::Eql(EqlTerm::Full(_)))]
            ),
            "expected $1 native and $2 EQL full payload, got: {:?}",
            typed.params
        );

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "INSERT INTO employees (id, salary) VALUES ($1, $2::JSONB::public.eql_v3_text) \
                 ON CONFLICT(id) DO UPDATE SET salary = excluded.salary"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `DO UPDATE ... WHERE` predicate is an ordinary predicate: a
    /// comparison over the encrypted column (on either the existing row or
    /// `excluded`) is rewritten through the ordering term.
    #[test]
    fn insert_on_conflict_do_update_where_rewrites_predicate() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            "INSERT INTO employees (id, salary) VALUES ($1, $2) \
             ON CONFLICT (id) DO UPDATE SET salary = excluded.salary \
             WHERE excluded.salary > employees.salary",
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "INSERT INTO employees (id, salary) VALUES ($1, $2::JSONB::public.eql_v3_text_ord) \
                 ON CONFLICT(id) DO UPDATE SET salary = excluded.salary \
                 WHERE eql_v3.ord_term(excluded.salary) > eql_v3.ord_term(employees.salary)"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// A conflict only fires off a unique index, and uniqueness of an
    /// encrypted column would be judged on the randomised ciphertext — the
    /// conflict would never fire. Rejected explicitly.
    #[test]
    fn insert_on_conflict_target_on_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq),
                }
            }
        });

        let statement = parse(
            "INSERT INTO employees (id, salary) VALUES (1, 2) \
             ON CONFLICT (salary) DO NOTHING",
        );

        assert!(
            type_check(schema, &statement).is_err(),
            "an encrypted conflict-target column should fail type checking"
        );
    }

    /// A window's `ORDER BY` sorts the partition, so an encrypted key is
    /// rewritten to its ordering term (`RewriteEqlOrderBy` fires on the
    /// `OrderByExpr` inside the window spec).
    #[test]
    fn window_order_by_encrypted_column_uses_ord_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        let statement = parse("SELECT rank() OVER (ORDER BY salary) FROM employees");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT rank() OVER (ORDER BY eql_v3.ord_term(salary)) FROM employees"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `Ord` bound on a window's `ORDER BY` key must reject a column whose
    /// domain carries no ordering term.
    #[test]
    fn window_order_by_requires_ord() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT rank() OVER (ORDER BY salary) FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "window ORDER BY over an Eq-only column should fail type checking"
        );
    }

    /// A named window definition (`WINDOW w AS (...)`) contains the same
    /// `WindowSpec` node as an inline `OVER (...)`, so `PARTITION BY` on an
    /// encrypted column is rewritten at the definition site and `OVER w` needs
    /// nothing of its own.
    #[test]
    fn named_window_partition_by_encrypted_column_uses_eq_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq),
                }
            }
        });

        let statement =
            parse("SELECT rank() OVER w FROM employees WINDOW w AS (PARTITION BY salary)");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT rank() OVER w FROM employees WINDOW w AS (PARTITION BY eql_v3.eq_term(salary))"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `Eq` bound applies inside a named window definition too. (`Ord`
    /// implies `Eq` in this model, so the rejection needs a storage-only
    /// domain.)
    #[test]
    fn named_window_partition_by_requires_eq() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    active (EQL("eql_v3_boolean")),
                }
            }
        });

        let statement =
            parse("SELECT rank() OVER w FROM employees WINDOW w AS (PARTITION BY active)");

        assert!(
            type_check(schema, &statement).is_err(),
            "named-window PARTITION BY over a storage-only column should fail type checking"
        );
    }

    /// A `RANGE` frame with an offset needs arithmetic on the sort key, which
    /// no term supports — rejected when the key is encrypted.
    #[test]
    fn range_offset_frame_over_encrypted_order_by_key_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = parse(
            "SELECT sum(id) OVER (ORDER BY salary RANGE BETWEEN 1 PRECEDING AND CURRENT ROW) \
             FROM employees",
        );

        assert!(
            type_check(schema, &statement).is_err(),
            "RANGE offset frame over an encrypted sort key should fail type checking"
        );
    }

    /// A `ROWS` frame counts rows, needing only the ordering the term
    /// provides — allowed, with the key rewritten.
    #[test]
    fn rows_offset_frame_over_encrypted_order_by_key_is_allowed() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        let statement = parse(
            "SELECT sum(id) OVER (ORDER BY salary ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) \
             FROM employees",
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT sum(id) OVER (ORDER BY eql_v3.ord_term(salary) ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) \
                 FROM employees"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// `count(DISTINCT enc)` dedupes by equality: the argument is rewritten to
    /// its equality term, which is deterministic per plaintext, so distinct
    /// terms count distinct plaintexts.
    #[test]
    fn count_distinct_encrypted_column_uses_eq_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT count(DISTINCT salary) FROM employees");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT count(DISTINCT eql_v3.eq_term(salary)) FROM employees"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `Eq` bound on `DISTINCT` aggregate arguments must reject a column
    /// whose domain carries no equality term at all. (`Ord` implies `Eq` in
    /// this model — equality falls back to the ordering term — so the
    /// rejection needs a storage-only domain.)
    #[test]
    fn count_distinct_requires_eq() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    active (EQL("eql_v3_boolean")),
                }
            }
        });

        let statement = parse("SELECT count(DISTINCT active) FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "count(DISTINCT ...) over a storage-only column should fail type checking"
        );
    }

    /// The equality-term substitution is only sound for `count`, which
    /// discards its argument values. Any other aggregate would have its result
    /// changed by the substitution, so it is rejected rather than silently
    /// miscomputed.
    #[test]
    fn min_distinct_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq + Ord),
                }
            }
        });

        let statement = parse("SELECT min(DISTINCT salary) FROM employees");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        assert!(
            matches!(
                typed.transform(HashMap::new()),
                Err(crate::EqlMapperError::Transform(_))
            ),
            "min(DISTINCT enc) should be rejected at transformation"
        );
    }

    /// An `ORDER BY` inside an aggregate's argument list sorts the values fed
    /// to the aggregate — the encrypted key is rewritten to its ordering term.
    #[test]
    fn aggregate_argument_order_by_encrypted_column_uses_ord_term() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL("eql_v3_integer_ord"): Ord),
                }
            }
        });

        let statement = parse("SELECT array_agg(id ORDER BY salary) FROM employees");

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {err:#?}"),
        };

        match typed.transform(HashMap::new()) {
            Ok(transformed_statement) => assert_eq!(
                transformed_statement.to_string(),
                "SELECT array_agg(id ORDER BY eql_v3.ord_term(salary)) FROM employees"
            ),
            Err(err) => panic!("statement transformation failed: {err}"),
        }
    }

    /// The `Ord` bound applies to the aggregate's argument-list `ORDER BY`.
    #[test]
    fn aggregate_argument_order_by_requires_ord() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Eq),
                }
            }
        });

        let statement = parse("SELECT array_agg(id ORDER BY salary) FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "aggregate ORDER BY over an Eq-only column should fail type checking"
        );
    }

    /// An ordered-set aggregate computes its result *from* the sort key, so an
    /// encrypted key cannot be rewritten to a term without handing the client
    /// the term itself. Rejected explicitly.
    #[test]
    fn within_group_on_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Ord),
                }
            }
        });

        let statement =
            parse("SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY salary) FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "WITHIN GROUP over an encrypted column should fail type checking"
        );
    }

    /// `WITHIN GROUP` over a native column remains supported.
    #[test]
    fn within_group_on_native_column_is_allowed() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Ord),
                }
            }
        });

        let statement =
            parse("SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY id) FROM employees");

        assert!(type_check(schema, &statement).is_ok());
    }

    /// `SELECT ... INTO` copies the projection into a table the schema has
    /// never seen: an encrypted column landing there would be unreachable
    /// ciphertext, so it is rejected.
    #[test]
    fn select_into_with_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse("SELECT salary INTO tmp_table FROM employees");

        assert!(
            type_check(schema, &statement).is_err(),
            "SELECT INTO projecting an encrypted column should fail type checking"
        );
    }

    /// `SELECT ... INTO` with only native columns passes through untouched.
    #[test]
    fn select_into_with_native_columns_is_allowed() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL),
                }
            }
        });

        let statement = parse("SELECT id INTO tmp_table FROM employees");

        assert!(type_check(schema, &statement).is_ok());
    }

    /// `ORDER BY ALL` (DuckDB/ClickHouse syntax) names every projected column
    /// without listing any expression to bound — rejected rather than left
    /// unconstrained. (The PostgreSQL dialect never parses it; this guards the
    /// AST shape itself.)
    #[test]
    fn order_by_all_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    salary (EQL: Ord),
                }
            }
        });

        let statement = sqltk::parser::parser::Parser::parse_sql(
            &sqltk::parser::dialect::DuckDbDialect {},
            "SELECT id, salary FROM employees ORDER BY ALL",
        )
        .unwrap()[0]
            .clone();

        assert!(
            type_check(schema, &statement).is_err(),
            "ORDER BY ALL should fail type checking"
        );
    }
}
