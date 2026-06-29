//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod dep;
mod display_helpers;
mod eql_mapper;
mod importer;
mod index_resolver;
mod inference;
mod iterator_ext;
mod model;
mod param;
mod scope_tracker;
mod ste_vec_ordering;
mod transformation_rules;
mod type_checked_statement;

#[cfg(test)]
mod test_helpers;

pub use display_helpers::*;
pub use eql_mapper::*;
pub use index_resolver::*;
pub use model::*;
pub use param::*;
pub use type_checked_statement::*;
pub use unifier::{
    Array, AssociatedType, EqlTerm, EqlTermVariant, EqlTrait, EqlTraits, EqlValue, NativeValue,
    Projection, ProjectionColumn, SetOf, TableColumn, Type, Value,
};

pub(crate) use dep::*;
pub(crate) use inference::*;
pub(crate) use scope_tracker::*;
pub(crate) use transformation_rules::*;

#[cfg(test)]
mod test {
    use super::{test_helpers::*, type_check, type_check_with_indexes};
    use crate::{
        projection, schema, test_helpers,
        unifier::{
            EqlTerm, EqlTrait, EqlTraits, EqlValue, InstantiateType, NativeValue, Projection,
            ProjectionColumn, Type, Value,
        },
        Param, Schema, TableColumn, TableResolver,
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
                        EqlTerm::Full(EqlValue(
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
                    EqlTerm::Full(EqlValue(
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
                    EqlTerm::Full(EqlValue(
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
                    EqlTerm::Full(EqlValue(
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
                    email (EQL),
                    first_name (EQL),
                }
            }
        });

        let statement = parse("select * from users WHERE email = $1 AND first_name = $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Eql(EqlTerm::Full(EqlValue(
                    TableColumn {
                        table: id("users"),
                        column: id("email"),
                    },
                    EqlTraits::default(),
                )));

                let b = Value::Eql(EqlTerm::Full(EqlValue(
                    TableColumn {
                        table: id("users"),
                        column: id("first_name"),
                    },
                    EqlTraits::default(),
                )));

                assert_eq!(typed.params, vec![(Param(1), a,), (Param(2), b,)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.email) as email),
                        (EQL(users.first_name) as first_name)
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
                let a = Value::Eql(EqlTerm::Full(EqlValue(
                    TableColumn {
                        table: id("users"),
                        column: id("salary"),
                    },
                    EqlTraits::from(EqlTrait::Ord),
                )));

                let b = Value::Eql(EqlTerm::Full(EqlValue(
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
                EqlTerm::Full(EqlValue(
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
                "SELECT * FROM employees WHERE salary > 'ENCRYPTED'::JSONB::eql_v2_encrypted"
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
                EqlTerm::Full(EqlValue(
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
                "INSERT INTO employees (salary) VALUES ('ENCRYPTED'::JSONB::eql_v2_encrypted)"
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
                    union
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
                    salary (EQL),
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
                        "SELECT eql_v2.min(salary), eql_v2.max(salary), department FROM employees GROUP BY department".to_string()
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
                    eql_col (EQL),
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
                            "SELECT * FROM employees WHERE eql_col = $1::JSONB::eql_v2_encrypted AND native_col = $2"
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
                            eql_v2.jsonb_path_exists(eql_col, '<encrypted-selector($.another-secret)>'::JSONB::eql_v2_encrypted), \
                            eql_v2.jsonb_path_query(eql_col, '<encrypted-selector($.secret)>'::JSONB::eql_v2_encrypted), \
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
                    format!("'<encrypted-selector({s})>'::JSONB::eql_v2_encrypted")
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
                    let rewritten_fn_name = format!("eql_v2.{fn_name}");
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
                            "@>" => "SELECT id, eql_v2.jsonb_contains(notes, '<encrypted-selector(medications)>'::JSONB::eql_v2_encrypted) AS meds FROM patients".to_string(),
                            "<@" => "SELECT id, eql_v2.jsonb_contained_by(notes, '<encrypted-selector(medications)>'::JSONB::eql_v2_encrypted) AS meds FROM patients".to_string(),
                            // Other operators are not transformed
                            _ => format!("SELECT id, notes {op} '<encrypted-selector(medications)>'::JSONB::eql_v2_encrypted AS meds FROM patients"),
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
            "SELECT id FROM patients WHERE eql_v2.jsonb_array(notes) @> eql_v2.jsonb_array(notes)",
        );

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v2.jsonb_array: {err}"),
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

        let statement = parse("SELECT id FROM patients WHERE eql_v2.jsonb_contains(notes, notes)");

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v2.jsonb_contains: {err}"),
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
            parse("SELECT id FROM patients WHERE eql_v2.jsonb_contained_by(notes, notes)");

        match type_check(schema, &statement) {
            Ok(_) => (),
            Err(err) => panic!("type check failed for eql_v2.jsonb_contained_by: {err}"),
        }
    }

    #[test]
    fn eql_v2_jsonb_contains_with_param() {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id,
                    notes (EQL: JsonLike + Contain),
                }
            }
        });

        let statement = parse("SELECT id FROM patients WHERE eql_v2.jsonb_contains(notes, $1)");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // Verify param was inferred as EQL type
        assert!(typed.params_contain_eql(), "param $1 should be EQL type");

        // Verify transformation output - function passes through, param gets cast
        match typed.transform(HashMap::new()) {
            Ok(statement) => assert_eq!(
                statement.to_string(),
                "SELECT id FROM patients WHERE eql_v2.jsonb_contains(notes, $1::JSONB::eql_v2_encrypted)"
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
            sql.contains("eql_v2.jsonb_contains"),
            "Expected @> to be transformed to eql_v2.jsonb_contains, got: {sql}"
        );

        // CRITICAL: Verify the parameter is cast to enable GIN index usage
        // The cast ::JSONB::eql_v2_encrypted is required for GIN indexes to work
        assert!(
            sql.contains("::JSONB::eql_v2_encrypted") || sql.contains("::jsonb::eql_v2_encrypted"),
            "Expected parameter to be cast as ::JSONB::eql_v2_encrypted for GIN index support, got: {sql}"
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
            sql.contains("eql_v2.jsonb_contained_by"),
            "Expected <@ to be transformed to eql_v2.jsonb_contained_by, got: {sql}"
        );

        // CRITICAL: Verify the parameter is cast to enable GIN index usage
        assert!(
            sql.contains("::JSONB::eql_v2_encrypted") || sql.contains("::jsonb::eql_v2_encrypted"),
            "Expected parameter to be cast as ::JSONB::eql_v2_encrypted for GIN index support, got: {sql}"
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
            sql.contains("eql_v2.jsonb_contains"),
            "Expected @> inside EXPLAIN to be transformed to eql_v2.jsonb_contains, got: {sql}"
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

        let statement = parse("SELECT eql_v2.jsonb_path_query(notes, $1) as notes FROM patients");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        assert_eq!(
            typed.projection,
            projection![(EQL(patients.notes: JsonLike) as notes)]
        );
    }

    /// Group B (CIP-3279): ordering comparisons on a jsonb sv-element extracted
    /// via `->` must be rewritten to compare CLLW ORE terms so the SQL binds to
    /// the `eql_v2.ore_cllw <op> eql_v2.ore_cllw` operators instead of the root
    /// Block-ORE (`ob`) path which raises `Expected an ore index (ob)`.
    #[test]
    fn jsonb_sv_ordering_rewrites_to_ore_cllw() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 > $2");

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT encrypted_jsonb FROM encrypted WHERE \
                        eql_v2.ore_cllw((encrypted_jsonb -> $1::JSONB::eql_v2_encrypted)::JSONB) > \
                        eql_v2.ore_cllw($2::JSONB)"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// The `jsonb_path_query_first` form of a jsonb sv ordering comparison must
    /// also rewrite to a CLLW ORE comparison. The function is first rewritten to
    /// `eql_v2.jsonb_path_query_first` by `RewriteStandardSqlFnsOnEqlTypes`.
    #[test]
    fn jsonb_sv_ordering_path_query_first_rewrites_to_ore_cllw() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement = parse(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) > $2",
        );

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT encrypted_jsonb FROM encrypted WHERE \
                        eql_v2.ore_cllw(eql_v2.jsonb_path_query_first(encrypted_jsonb, $1::JSONB::eql_v2_encrypted)::JSONB) > \
                        eql_v2.ore_cllw($2::JSONB)"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// Each ordering operator (`<`, `<=`, `>`, `>=`) on a jsonb sv element must
    /// rewrite to the CLLW ORE comparison preserving the operator.
    #[test]
    fn jsonb_sv_ordering_all_operators_rewrite_to_ore_cllw() {
        for op in ["<", "<=", ">", ">="] {
            let schema = resolver(schema! {
                tables: {
                    encrypted: {
                        id,
                        encrypted_jsonb (EQL: JsonLike),
                    }
                }
            });

            let statement = parse(&format!(
                "SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 {op} $2"
            ));

            match type_check(schema, &statement) {
                Ok(typed) => match typed.transform(HashMap::new()) {
                    Ok(statement) => {
                        assert_eq!(
                            statement.to_string(),
                            format!(
                                "SELECT encrypted_jsonb FROM encrypted WHERE \
                                eql_v2.ore_cllw((encrypted_jsonb -> $1::JSONB::eql_v2_encrypted)::JSONB) {op} \
                                eql_v2.ore_cllw($2::JSONB)"
                            )
                        );
                    }
                    Err(err) => panic!("transformation failed for `{op}`: {err}"),
                },
                Err(err) => panic!("type check failed for `{op}`: {err}"),
            }
        }
    }

    /// Equality (`=`) on a jsonb sv element must NOT be rewritten to a CLLW ORE
    /// comparison: equality is hmac/oc-based and resolves through the
    /// `eql_v2.eq_term` path, not the ordering `eql_v2.ore_cllw` path. This
    /// guards against the ordering rule over-matching the equality operators.
    #[test]
    fn jsonb_sv_equality_is_not_rewritten_to_ore_cllw() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2");

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    let sql = statement.to_string();
                    assert!(
                        !sql.contains("ore_cllw"),
                        "equality must not bind to ore_cllw, got: {sql}"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// Group C (CIP-3281): equality (`=`) on a jsonb sv-element extracted via
    /// `->` must be rewritten to compare the XOR-aware equality terms so the SQL
    /// binds to `eql_v2.eq_term` rather than the root `eql_v2_encrypted`
    /// equality path. The left operand (`col -> sel`) is an
    /// `eql_v2.ste_vec_entry`, so `eql_v2.eq_term(...)` reads its `hm`/`oc`
    /// term; the right operand is the query payload jsonb, whose `hm`/`oc` term
    /// is read with the inlined `eq_term` body (`decode(coalesce(...), 'hex')`).
    #[test]
    fn jsonb_sv_equality_rewrites_to_eq_term() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2");

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT encrypted_jsonb FROM encrypted WHERE \
                        eql_v2.eq_term(encrypted_jsonb -> $1::JSONB::eql_v2_encrypted) = \
                        decode(coalesce($2::JSONB ->> 'hm', $2::JSONB ->> 'oc'), 'hex')"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// Inequality (`<>`) on a jsonb sv element must rewrite to the same
    /// `eql_v2.eq_term` comparison, preserving the `<>` operator.
    #[test]
    fn jsonb_sv_inequality_rewrites_to_eq_term() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 <> $2");

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT encrypted_jsonb FROM encrypted WHERE \
                        eql_v2.eq_term(encrypted_jsonb -> $1::JSONB::eql_v2_encrypted) <> \
                        decode(coalesce($2::JSONB ->> 'hm', $2::JSONB ->> 'oc'), 'hex')"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// The `jsonb_path_query_first` form of a jsonb sv equality comparison must
    /// also rewrite to the `eql_v2.eq_term` path. The function is first
    /// rewritten to `eql_v2.jsonb_path_query_first` by
    /// `RewriteStandardSqlFnsOnEqlTypes`; its result is an `eql_v2_encrypted`,
    /// so it is cast to `::eql_v2.ste_vec_entry` (via jsonb) before `eq_term`.
    #[test]
    fn jsonb_sv_equality_path_query_first_rewrites_to_eq_term() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement = parse(
            "SELECT encrypted_jsonb FROM encrypted WHERE jsonb_path_query_first(encrypted_jsonb, $1) = $2",
        );

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT encrypted_jsonb FROM encrypted WHERE \
                        eql_v2.eq_term(eql_v2.jsonb_path_query_first(encrypted_jsonb, $1::JSONB::eql_v2_encrypted)::JSONB::eql_v2.ste_vec_entry) = \
                        decode(coalesce($2::JSONB ->> 'hm', $2::JSONB ->> 'oc'), 'hex')"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// The RHS param of a jsonb sv equality comparison must resolve to
    /// `EqlTerm::SteVecTerm` so the proxy encrypts it as the matching STE-vec
    /// equality term (`hm`/`oc`) carried by the column's leaf.
    #[test]
    fn jsonb_sv_equality_rhs_param_is_ste_vec_term() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 = $2");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        let (_, value) = typed
            .params
            .iter()
            .find(|(p, _)| *p == Param(2))
            .expect("param $2 should be present");

        assert!(
            matches!(value, Value::Eql(EqlTerm::SteVecTerm(_))),
            "expected $2 to be EqlTerm::SteVecTerm, got {value}"
        );
    }

    /// The RHS *literal* of a jsonb sv equality comparison must also resolve to
    /// `EqlTerm::SteVecTerm` so a simple-protocol query (inline literal) is
    /// encrypted as the matching STE-vec equality term.
    #[test]
    fn jsonb_sv_equality_rhs_literal_is_ste_vec_term() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' = 4");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        let ste_vec_term_count = typed
            .literals
            .iter()
            .filter(|(eql_term, _)| matches!(eql_term, EqlTerm::SteVecTerm(_)))
            .count();

        assert_eq!(
            ste_vec_term_count, 1,
            "expected exactly one SteVecTerm literal (the comparison value), got {:?}",
            typed.literals
        );
    }

    /// Security regression: the inline RHS *literal* of a jsonb sv equality
    /// comparison must be replaced by its encrypted ciphertext in the
    /// transformed SQL — the plaintext value must never survive into the query
    /// the proxy sends downstream.
    ///
    /// Because the transform walks bottom-up (children before parents), the RHS
    /// literal node is cast to `<ct>::JSONB::eql_v2_encrypted` by
    /// `CastLiteralsAsEncrypted` *before* `RewriteJsonbSteVecEquality` rewrites
    /// the parent comparison. The equality rule then strips the outer
    /// `::eql_v2_encrypted` cast (via `rhs_as_jsonb`) and reads `->> 'hm'/'oc'`
    /// off the encrypted payload. The clone inside `build_eq_term_rhs` operates
    /// on that already-encrypted output node, not on the original literal node
    /// `CastLiteralsAsEncrypted` keys on, so it cannot leave the plaintext
    /// uncast.
    #[test]
    fn jsonb_sv_equality_rhs_literal_is_encrypted_in_sql() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' = 4");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // Encrypt every literal to a distinguishable marker so we can assert the
        // RHS comparison value (`4`) is encrypted, not left as plaintext.
        let encrypted_literals: HashMap<NodeKey<'_>, ast::Value> = typed
            .literals
            .iter()
            .map(|(_, node)| {
                (
                    node.as_node_key(),
                    ast::Value::SingleQuotedString(format!("ENC({node})")),
                )
            })
            .collect();

        let sql = typed
            .transform(encrypted_literals)
            .map_err(|err| err.to_string())
            .unwrap()
            .to_string();

        // The RHS comparison value must appear as its encrypted ciphertext,
        // wrapped in the `::JSONB` cast the inlined `eq_term` body reads from.
        assert!(
            sql.contains("'ENC(4)'::JSONB ->> 'hm'") && sql.contains("'ENC(4)'::JSONB ->> 'oc'"),
            "the RHS literal `4` must be encrypted in the SQL, got: {sql}"
        );

        // The plaintext value `4` must never survive bare in the SQL. Strip the
        // encrypted-ciphertext markers first, then assert the digit `4` does not
        // appear as a standalone numeric token anywhere in what remains. This is
        // robust to formatting changes (unlike matching specific `= 4` spellings).
        let without_ciphertext = sql.replace("'ENC(4)'", "");
        let bare_number_4 = without_ciphertext
            .split(|c: char| !c.is_ascii_digit())
            .any(|token| token == "4");
        assert!(
            !bare_number_4,
            "the plaintext value `4` must not appear bare in the SQL, got: {sql}"
        );
    }

    /// The commutative form with the accessor on the *right*
    /// (`value = col -> selector`) is intentionally NOT rewritten: the rule and
    /// the reclassification both gate on `is_ste_vec_accessor(left)`, so they
    /// agree to leave it as root `eql_v2_encrypted` equality. This guards that
    /// intentional left-operand-only behaviour so a future change that "adds
    /// commutativity" to only one of the two passes is caught.
    #[test]
    fn jsonb_sv_equality_commutative_form_is_not_rewritten() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE $2 = encrypted_jsonb -> $1");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // The rewrite must not fire: no `eq_term` in the emitted SQL.
        let sql = typed.transform(HashMap::new()).unwrap().to_string();
        assert!(
            !sql.contains("eq_term"),
            "commutative form must not be rewritten to eq_term, got: {sql}"
        );

        // And the param must NOT be reclassified to SteVecTerm — the two passes
        // must stay in lockstep.
        let (_, value) = typed
            .params
            .iter()
            .find(|(p, _)| *p == Param(2))
            .expect("param $2 should be present");
        assert!(
            !matches!(value, Value::Eql(EqlTerm::SteVecTerm(_))),
            "commutative-form RHS must NOT be reclassified to SteVecTerm, got {value}"
        );
    }

    /// The RHS param of a jsonb sv ordering comparison must resolve to
    /// `EqlTerm::SteVecTerm` so the proxy encrypts it as a CLLW ORE STE-vec
    /// query term (`oc`).
    #[test]
    fn jsonb_sv_ordering_rhs_param_is_ste_vec_term() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> $1 > $2");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // $2 is the comparison RHS; it must be a SteVecTerm.
        let (_, value) = typed
            .params
            .iter()
            .find(|(p, _)| *p == Param(2))
            .expect("param $2 should be present");

        assert!(
            matches!(value, Value::Eql(EqlTerm::SteVecTerm(_))),
            "expected $2 to be EqlTerm::SteVecTerm, got {value}"
        );
    }

    /// The RHS *literal* of a jsonb sv ordering comparison must also resolve to
    /// `EqlTerm::SteVecTerm` so a simple-protocol query (inline literal) is
    /// encrypted as a CLLW ORE STE-vec query term (`oc`).
    #[test]
    fn jsonb_sv_ordering_rhs_literal_is_ste_vec_term() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' > 4");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        // The literals are (selector, comparison-value). The comparison value
        // (`4`) must be a SteVecTerm; the selector (`'number'`) is a JsonAccessor.
        let ste_vec_term_count = typed
            .literals
            .iter()
            .filter(|(eql_term, _)| matches!(eql_term, EqlTerm::SteVecTerm(_)))
            .count();

        assert_eq!(
            ste_vec_term_count, 1,
            "expected exactly one SteVecTerm literal (the comparison value), got {:?}",
            typed.literals
        );
    }

    /// A *negative* numeric RHS literal (`-70`) parses as
    /// `Expr::UnaryOp { Minus, Number }`, which the type inferencer forces to
    /// `Native` (see `Expr::UnaryOp` in `infer_type_impls/expr.rs`). It
    /// therefore cannot unify against an EQL operand, so a jsonb sv comparison
    /// with a negative literal RHS is *rejected at type-check time* rather than
    /// being rewritten.
    ///
    /// This is the safety-critical guard the STE-vec rewrites depend on: because
    /// the UnaryOp RHS is `Native`, the rewrite rules' `is_eql_typed(right)`
    /// gate is `false`, so the SQL is **not** rewritten to `eql_v2.ore_cllw` /
    /// `eql_v2.eq_term`. The dangerous "SQL rewritten as an sv term but the
    /// value not reclassified as `SteVecTerm`" mismatch is structurally
    /// impossible. (This UnaryOp→Native limitation is pre-existing and applies
    /// identically to scalar EQL comparisons such as `encrypted_int > -70`.)
    #[test]
    fn jsonb_sv_ordering_rhs_negative_literal_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' > -70");

        // Assert it is rejected *for the right reason*: a Native/EQL unification
        // failure (the UnaryOp RHS forced to Native), not some unrelated error.
        let err = type_check(schema, &statement)
            .expect_err(
                "negative-literal jsonb sv ordering comparison must be rejected at type-check time",
            )
            .to_string();

        assert!(
            err.contains("unify") && err.contains("Native"),
            "expected a Native/EQL unification failure (UnaryOp RHS is Native), got: {err}"
        );
    }

    /// Equality counterpart of [`jsonb_sv_ordering_rhs_negative_literal_is_rejected`]:
    /// a negative-literal RHS of a jsonb sv equality comparison is likewise
    /// rejected at type-check time, so it is never rewritten to `eql_v2.eq_term`
    /// against an unreclassified value.
    #[test]
    fn jsonb_sv_equality_rhs_negative_literal_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT encrypted_jsonb FROM encrypted WHERE encrypted_jsonb -> 'number' = -70");

        // Assert it is rejected *for the right reason*: a Native/EQL unification
        // failure (the UnaryOp RHS forced to Native), not some unrelated error.
        let err = type_check(schema, &statement)
            .expect_err(
                "negative-literal jsonb sv equality comparison must be rejected at type-check time",
            )
            .to_string();

        assert!(
            err.contains("unify") && err.contains("Native"),
            "expected a Native/EQL unification failure (UnaryOp RHS is Native), got: {err}"
        );
    }

    /// The RHS param of a root-scalar ordering comparison must remain
    /// `EqlTerm::Partial` (root Block-ORE path), NOT be reclassified.
    #[test]
    fn root_scalar_ordering_rhs_param_is_partial() {
        use crate::unifier::EqlTerm;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int > $1");

        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        let (_, value) = typed
            .params
            .iter()
            .find(|(p, _)| *p == Param(1))
            .expect("param $1 should be present");

        assert!(
            !matches!(value, Value::Eql(EqlTerm::SteVecTerm(_))),
            "root-scalar ordering RHS must NOT be reclassified to SteVecTerm, got {value}"
        );
    }

    /// A root-scalar ordering comparison (not a jsonb sv accessor) must NOT be
    /// rewritten — it relies on the root Block-ORE (`ob`) operators.
    #[test]
    fn root_scalar_ordering_is_not_rewritten_to_ore_cllw() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int > $1");

        match type_check(schema, &statement) {
            Ok(typed) => match typed.transform(HashMap::new()) {
                Ok(statement) => {
                    assert_eq!(
                        statement.to_string(),
                        "SELECT id FROM encrypted WHERE encrypted_int > $1::JSONB::eql_v2_encrypted"
                    );
                }
                Err(err) => panic!("transformation failed: {err}"),
            },
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    /// A jsonb sv ordering comparison whose RHS is a *different* encrypted
    /// column (`accessor <op> other_encrypted -> sel`) is rejected by EQL term
    /// unification — EQL ciphertexts are column-scoped, so two distinct columns
    /// never unify. This is why the ordering rewrite's `is_eql_typed(right)`
    /// gate is sufficient: the only EQL RHS that can reach the rewrite is one
    /// that unified to the *same* `TableColumn` (a literal/param, or the same
    /// column re-referenced), which carries the matching sv term. A mismatched
    /// "other encrypted column on the RHS" shape is not representable.
    #[test]
    fn jsonb_sv_ordering_rhs_different_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_jsonb (EQL: JsonLike),
                    other_jsonb (EQL: JsonLike),
                }
            }
        });

        let statement =
            parse("SELECT id FROM encrypted WHERE encrypted_jsonb -> 'n' > other_jsonb -> 'n'");

        let err = type_check(schema, &statement)
            .expect_err("comparing two different encrypted columns must be rejected")
            .to_string();

        assert!(
            err.contains("unify") && err.contains("EQL"),
            "expected an EQL-term unification failure between the two columns, got: {err}"
        );
    }

    /// Scalar OPE counterpart of
    /// [`jsonb_sv_ordering_rhs_different_encrypted_column_is_rejected`]: a scalar
    /// ordering comparison between two *different* encrypted columns is rejected
    /// by EQL term unification, so the scalar OPE rewrite's `is_eql_typed(right)`
    /// gate can never bind a non-OPE foreign column on the RHS.
    #[test]
    fn scalar_ordering_rhs_different_encrypted_column_is_rejected() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    a (EQL: Ord),
                    b (EQL: Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM encrypted WHERE a > b");

        let err = type_check(schema, &statement)
            .expect_err("comparing two different encrypted columns must be rejected")
            .to_string();

        assert!(
            err.contains("unify") && err.contains("EQL"),
            "expected an EQL-term unification failure between the two columns, got: {err}"
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

    /// A scalar (non-jsonb) `eql_v2_encrypted` column whose resolved index set
    /// contains `Ope` must have ordering comparisons rewritten to compare the
    /// order-preserving `op` ciphertext directly using Postgres built-ins.
    ///
    /// `col <op> $param`  →  `decode(col->>'op','hex') <op> decode($param->>'op','hex')`
    #[test]
    fn scalar_ope_ordering_rewrites_to_decode_op() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        for op in ["<", "<=", ">", ">="] {
            let schema = resolver(schema! {
                tables: {
                    encrypted: {
                        id,
                        encrypted_int (EQL: Ord),
                    }
                }
            });

            let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
                TableColumn {
                    table: id("encrypted"),
                    column: id("encrypted_int"),
                },
                HashSet::from_iter([IndexKind::Ope]),
            )])));

            let statement = parse(&format!(
                "SELECT id FROM encrypted WHERE encrypted_int {op} $1"
            ));

            let typed = type_check_with_indexes(schema, &statement, index_resolver)
                .map_err(|err| err.to_string())
                .unwrap();

            let transformed = typed.transform(HashMap::new()).unwrap();

            assert_eq!(
                transformed.to_string(),
                format!(
                    "SELECT id FROM encrypted WHERE \
                    decode(encrypted_int::JSONB ->> 'op', 'hex') {op} \
                    decode($1::JSONB::eql_v2_encrypted::JSONB ->> 'op', 'hex')"
                ),
                "operator `{op}` must rewrite to a decode(op) byte comparison"
            );
        }
    }

    /// `ORDER BY col [ASC|DESC] [NULLS …]` on a scalar OPE column must rewrite
    /// the sort key to `decode(col->>'op','hex')`, preserving direction and
    /// nulls ordering.
    #[test]
    fn scalar_ope_order_by_rewrites_to_decode_op() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let cases = [
            (
                "ORDER BY encrypted_int",
                "ORDER BY decode(encrypted_int::JSONB ->> 'op', 'hex')",
            ),
            (
                "ORDER BY encrypted_int ASC",
                "ORDER BY decode(encrypted_int::JSONB ->> 'op', 'hex') ASC",
            ),
            (
                "ORDER BY encrypted_int DESC",
                "ORDER BY decode(encrypted_int::JSONB ->> 'op', 'hex') DESC",
            ),
            (
                "ORDER BY encrypted_int DESC NULLS LAST",
                "ORDER BY decode(encrypted_int::JSONB ->> 'op', 'hex') DESC NULLS LAST",
            ),
            (
                "ORDER BY encrypted_int ASC NULLS FIRST",
                "ORDER BY decode(encrypted_int::JSONB ->> 'op', 'hex') ASC NULLS FIRST",
            ),
        ];

        for (order_by, expected_order_by) in cases {
            let schema = resolver(schema! {
                tables: {
                    encrypted: {
                        id,
                        encrypted_int (EQL: Ord),
                    }
                }
            });

            let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
                TableColumn {
                    table: id("encrypted"),
                    column: id("encrypted_int"),
                },
                HashSet::from_iter([IndexKind::Ope]),
            )])));

            let statement = parse(&format!("SELECT id FROM encrypted {order_by}"));

            let typed = type_check_with_indexes(schema, &statement, index_resolver)
                .map_err(|err| err.to_string())
                .unwrap();

            let transformed = typed.transform(HashMap::new()).unwrap();

            assert_eq!(
                transformed.to_string(),
                format!("SELECT id FROM encrypted {expected_order_by}"),
                "`{order_by}` must rewrite the sort key to decode(op)"
            );
        }
    }

    /// An ORE-only scalar column (resolved index set has `Ore`, no `Ope`) must
    /// NOT be rewritten to the `decode(op)` form: it stays on the existing
    /// root Block-ORE bare-operator path.
    #[test]
    fn scalar_ore_only_ordering_is_not_rewritten_to_decode_op() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ore]),
        )])));

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int > $1");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted WHERE encrypted_int > $1::JSONB::eql_v2_encrypted",
            "ORE-only column must remain on the bare-operator path"
        );
    }

    /// With no concrete index information (empty resolver, the default), a
    /// scalar ordering comparison must NOT be rewritten to `decode(op)`: it
    /// retains today's behaviour. This is the guard that the default/empty
    /// resolver reproduces existing behaviour.
    #[test]
    fn scalar_ordering_with_empty_resolver_is_not_rewritten() {
        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int > $1");

        // `type_check` uses the empty resolver by default.
        let typed = type_check(schema, &statement)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted WHERE encrypted_int > $1::JSONB::eql_v2_encrypted",
            "empty resolver must reproduce today's behaviour"
        );
    }

    /// Equality (`=`) on a scalar OPE column must NOT be rewritten by the
    /// ordering rule (equality is a separate concern, CIP-3281).
    #[test]
    fn scalar_ope_equality_is_not_rewritten_to_decode_op() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Eq + Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ope, IndexKind::Unique]),
        )])));

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int = $1");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted WHERE encrypted_int = $1::JSONB::eql_v2_encrypted",
            "equality must not be rewritten by the OPE ordering rule"
        );
    }

    /// Pins the *actual* behaviour for column-on-the-right (`$param <op> col`).
    ///
    /// The rule's `would_edit_comparison` guard reads `is_scalar_ope_column(left)
    /// && is_eql_typed(right)`, which textually looks like it only matches the
    /// EQL column on the left. In practice a bare comparison param is unified to
    /// the *same* `eql_v2_encrypted` column type, so `is_scalar_ope_column`
    /// resolves the param's `TableColumn` to the OPE-indexed column and the rule
    /// fires for *either* operand ordering. Both sides are therefore wrapped in
    /// `decode(... ->> 'op', 'hex')`. The operator itself is unchanged (`<` stays
    /// `<`); correctness rests on `decode(... ->> 'op')` being an order-preserving
    /// transform applied to *both* operands, so the comparison still evaluates the
    /// same OPE ordering it would have on the unwrapped values.
    ///
    /// This differs from `RewriteJsonbSteVecOrdering`, where the jsonb accessor
    /// (`col -> selector`) only appears syntactically on one side. Here there is
    /// no coverage gap to pin — column-on-the-right is handled. Recorded as a
    /// test so the behaviour is locked in rather than incidental.
    #[test]
    fn scalar_ope_ordering_column_on_right_is_rewritten() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ope]),
        )])));

        let statement = parse("SELECT id FROM encrypted WHERE $1 < encrypted_int");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted WHERE \
            decode($1::JSONB::eql_v2_encrypted::JSONB ->> 'op', 'hex') < \
            decode(encrypted_int::JSONB ->> 'op', 'hex')",
            "column-on-the-right comparison is rewritten symmetrically to decode(op)"
        );
    }

    /// Pins an accepted limitation: `col BETWEEN $a AND $b` is a distinct AST
    /// node (`Expr::Between`), not the `Expr::BinaryOp` the OPE ordering rule
    /// matches, so it is NOT rewritten to the `decode(op)` form.
    #[test]
    fn scalar_ope_ordering_between_is_not_rewritten() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ope]),
        )])));

        let statement = parse("SELECT id FROM encrypted WHERE encrypted_int BETWEEN $1 AND $2");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted WHERE encrypted_int BETWEEN \
            $1::JSONB::eql_v2_encrypted AND $2::JSONB::eql_v2_encrypted",
            "BETWEEN must not be rewritten by the OPE ordering rule"
        );
    }

    /// Pins an accepted limitation: `MIN(col)` / `MAX(col)` are aggregate
    /// function calls, not ordering comparisons, so the OPE ordering rule does
    /// not touch them. They are instead routed to the `eql_v2.min` / `eql_v2.max`
    /// EQL aggregate functions by a separate rule.
    #[test]
    fn scalar_ope_min_max_are_not_rewritten_to_decode_op() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ope]),
        )])));

        let statement = parse("SELECT min(encrypted_int), max(encrypted_int) FROM encrypted");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT eql_v2.min(encrypted_int), eql_v2.max(encrypted_int) FROM encrypted",
            "MIN/MAX must route to eql_v2 aggregates, not decode(op)"
        );
    }

    /// A multi-key `ORDER BY ope_col, plaintext_col` must rewrite *only* the
    /// OPE sort key to `decode(col->>'op','hex')`, leaving the plaintext key
    /// untouched and preserving each key's direction / nulls ordering.
    #[test]
    fn scalar_ope_multi_key_order_by_rewrites_only_ope_key() {
        use crate::{IndexKind, MapIndexResolver};
        use std::collections::HashSet;

        let schema = resolver(schema! {
            tables: {
                encrypted: {
                    id,
                    encrypted_int (EQL: Ord),
                }
            }
        });

        let index_resolver = Arc::new(MapIndexResolver::new(HashMap::from_iter([(
            TableColumn {
                table: id("encrypted"),
                column: id("encrypted_int"),
            },
            HashSet::from_iter([IndexKind::Ope]),
        )])));

        let statement =
            parse("SELECT id FROM encrypted ORDER BY encrypted_int DESC NULLS LAST, id ASC");

        let typed = type_check_with_indexes(schema, &statement, index_resolver)
            .map_err(|err| err.to_string())
            .unwrap();

        let transformed = typed.transform(HashMap::new()).unwrap();

        assert_eq!(
            transformed.to_string(),
            "SELECT id FROM encrypted ORDER BY \
            decode(encrypted_int::JSONB ->> 'op', 'hex') DESC NULLS LAST, id ASC",
            "only the OPE sort key is rewritten; plaintext key and directions are preserved"
        );
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
