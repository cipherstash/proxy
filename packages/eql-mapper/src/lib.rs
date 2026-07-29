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
        JsonSelectorSource, OutputParamSource, Param, Schema, TableColumn, TableResolver,
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
                    salary (EQL),
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
                (EQL(employees.salary) as salary),
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
                    salary (EQL),
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
                (EQL(employees.salary) as salary),
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
                    salary (EQL),
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
            projection![(NATIVE as count), (EQL(employees.salary) as salary)]
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
                path: JsonSelectorSource::Param(Param(1)),
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
                path: JsonSelectorSource::Param(Param(2)),
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

    /// Transforms `sql` with every encrypted literal replaced by `'<CT>'`.
    fn transform_with_dummy_literals(schema: Arc<TableResolver>, sql: &str) -> String {
        let statement = parse(sql);
        let typed = type_check(schema, &statement).unwrap();

        let encrypted = typed
            .literals
            .iter()
            .map(|(_, v)| {
                (
                    NodeKey::new(*v),
                    ast::Value::SingleQuotedString("<CT>".to_string()),
                )
            })
            .collect::<HashMap<_, _>>();

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

    /// Shapes that sort, group or deduplicate an encrypted column through
    /// PostgreSQL's operator class rather than through an operator.
    ///
    /// None is rewritten, so each reaches jsonb's own btree/hash operator class
    /// and compares whole payloads — including the randomised ciphertext. The
    /// assertion is deliberately a necessary condition rather than an exact
    /// string: whatever form the fix takes, the encrypted column cannot reach
    /// the clause unwrapped. Rejecting the shape loudly is an equally acceptable
    /// outcome, in which case these should be rewritten to assert the error.
    #[test]
    #[ignore = "None of these shapes is rewritten: DISTINCT ON, ordinal ORDER BY/GROUP BY, \
                PARTITION BY and set operations all reach jsonb's operator class, comparing raw \
                payloads. Each needs the term-based rewrite its named-column equivalent already \
                has, or a loud rejection."]
    fn operator_class_shapes_use_the_columns_term() {
        let schema = forms_schema();

        for (input, required_term) in [
            ("SELECT DISTINCT ON (txt) txt FROM t", "eql_v3.eq_term("),
            ("SELECT txt FROM t ORDER BY 1", "eql_v3.ord_term("),
            ("SELECT txt FROM t GROUP BY 1", "eql_v3.eq_term("),
            (
                "SELECT rank() OVER (PARTITION BY txt) FROM t",
                "eql_v3.eq_term(",
            ),
        ] {
            let rewritten = transform_with_dummy_literals(schema.clone(), input);

            assert!(
                rewritten.contains(required_term),
                "`{input}` must compare the column by `{required_term}…)`, got: {rewritten}"
            );
        }
    }

    /// The same shapes must refuse a column that cannot support them.
    #[test]
    #[ignore = "None of these shapes applies a capability bound, so a storage-only column is \
                accepted and silently mis-sorted or mis-grouped. Fixing the rewrites should \
                come with the matching bound, as SELECT DISTINCT and DISTINCT ON already have."]
    fn operator_class_shapes_require_their_capability() {
        let schema = forms_schema();

        for (input, bound) in [
            ("SELECT flag FROM t ORDER BY 1", "Ord"),
            ("SELECT flag FROM t GROUP BY 1", "Eq"),
            ("SELECT rank() OVER (PARTITION BY flag) FROM t", "Eq"),
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

    /// A chained JSON accessor must not leave the intermediate selector in the
    /// statement, nor apply native `->` to the encrypted payload.
    ///
    /// The container is cloned from the *original* AST, so `-> 'nested'`
    /// survives untouched: the plaintext field name ships in the SQL text and
    /// native jsonb `->` runs on the encrypted column, which also makes the
    /// predicate match nothing.
    #[test]
    #[ignore = "Chained JSON accessor clones its container from the original AST, so the inner \
                selector stays plaintext in the SQL and native jsonb -> is applied to the \
                encrypted payload. See rewrite_json_value_selector_eq.rs."]
    fn chained_json_accessor_does_not_emit_the_plaintext_selector() {
        let schema = resolver(schema! {
            tables: {
                t: {
                    id,
                    j (EQL("eql_v3_json_search"): Eq + Ord + JsonLike + Contain),
                }
            }
        });

        let rewritten = transform_with_dummy_literals(
            schema,
            "SELECT id FROM t WHERE j -> 'nested' -> 'string' = '\"world\"'",
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

        let statement = parse("SELECT COUNT(*) FROM employees GROUP BY name");
        let typed = type_check(schema, &statement).unwrap();

        let err = typed.transform(HashMap::new()).unwrap_err().to_string();
        assert!(
            err.contains("GROUP BY") && err.contains("no equality term"),
            "expected a capability error, got: {err}"
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

        let statement = parse("SELECT id FROM employees ORDER BY name");
        let typed = type_check(schema, &statement).unwrap();

        let err = typed.transform(HashMap::new()).unwrap_err().to_string();
        assert!(
            err.contains("ORDER BY") && err.contains("no ordering term"),
            "expected a capability error, got: {err}"
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
}
