//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod dep;
mod eql_mapper;
mod importer;
mod inference;
mod iterator_ext;
mod model;
mod scope_tracker;

#[cfg(test)]
mod test_helpers;

pub use dep::*;
pub use eql_mapper::*;
pub use importer::*;
pub use inference::*;
pub use model::*;
pub use scope_tracker::*;
pub use unifier::{EqlValue, NativeValue, TableColumn};

#[cfg(test)]
mod test {
    use super::test_helpers::*;
    use super::type_check;
    use crate::col;
    use crate::projection;
    use crate::Schema;
    use crate::TableResolver;
    use crate::{schema, EqlValue, NativeValue, Projection, ProjectionColumn, TableColumn, Value};
    use pretty_assertions::assert_eq;
    use sqlparser::ast::Statement;
    use sqlparser::ast::{self as ast};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn resolver(schema: Schema) -> Arc<TableResolver> {
        Arc::new(TableResolver::new_fixed(schema.into()))
    }

    #[test]
    fn basic() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
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
                    Some(projection![(NATIVE(users.email) as email)])
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn basic_with_value() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("select email from users WHERE email = 'hello@cipherstash.com'");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.projection,
                    Some(projection![(EQL(users.email) as email)])
                );

                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Expr::Value(ast::Value::SingleQuotedString(
                        "hello@cipherstash.com".into()
                    ))
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn insert_with_value() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users (id, email) VALUES (42, 'hello@cipherstash.com')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                eprintln!("{:#?}", &typed.literals);
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Expr::Value(ast::Value::SingleQuotedString(
                        "hello@cipherstash.com".into()
                    ))
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn insert_with_values_no_explicit_columns() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users VALUES (42, 'hello@cipherstash.com', 'James')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                eprintln!("{:#?}", &typed.literals);
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Expr::Value(ast::Value::SingleQuotedString(
                        "hello@cipherstash.com".into()
                    ))
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn insert_with_values_no_explicit_columns_but_has_default() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement =
            parse("INSERT INTO users VALUES (default, 'hello@cipherstash.com', 'James')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                eprintln!("{:#?}", &typed.literals);
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Expr::Value(ast::Value::SingleQuotedString(
                        "hello@cipherstash.com".into()
                    ))
                )));
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn basic_with_placeholder() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users WHERE id = $1");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let v = Value::Native(NativeValue(Some(TableColumn {
                    table: id("users"),
                    column: id("id"),
                })));

                let param = typed.params.first().unwrap();

                assert_eq!(param, &v);

                assert_eq!(
                    typed.projection,
                    Some(projection![(NATIVE(users.email) as email)])
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_placeholder() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
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

                assert_eq!(typed.params, vec![a, b]);

                assert_eq!(
                    typed.projection,
                    Some(projection![
                        (NATIVE(users.id) as id),
                        (NATIVE(users.email) as email),
                        (NATIVE(users.first_name) as first_name)
                    ])
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_instances_of_placeholder() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
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

                assert_eq!(typed.params, vec![a]);

                assert_eq!(
                    typed.projection,
                    Some(projection![
                        (NATIVE(users.id) as id),
                        (NATIVE(users.email) as email),
                        (NATIVE(users.first_name) as first_name)
                    ])
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_multiple_tables() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
                todo_lists: {
                    id (PK),
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
                assert_eq!(
                    typed.projection,
                    Some(projection![(EQL(users.email) as email)])
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_subquery() {
        let _ = tracing_subscriber::fmt::try_init();
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
            Some(projection![
                (NATIVE as user_id),
                (NATIVE as todo_list_item_id),
                (EQL(todo_list_items.description) as todo_list_item_description)
            ])
        );
    }

    #[test]
    fn wildcard_expansion() {
        let _ = tracing_subscriber::fmt::try_init();
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(users.id) as id),
                (EQL(users.email) as email),
                (NATIVE(todo_lists.id) as id),
                (NATIVE(todo_lists.owner_id) as owner_id),
                (EQL(todo_lists.secret) as secret)
            ])
        );
    }

    #[test]
    fn select_with_multiple_placeholder_and_wildcard_expansion() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name (EQL),
                }
            }
        });

        let statement = parse("select * from users WHERE email = $1 AND first_name = $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Eql(EqlValue(TableColumn {
                    table: id("users"),
                    column: id("email"),
                }));

                let b = Value::Eql(EqlValue(TableColumn {
                    table: id("users"),
                    column: id("first_name"),
                }));

                assert_eq!(typed.params, vec![a, b]);

                assert_eq!(
                    typed.projection,
                    Some(projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.email) as email),
                        (EQL(users.first_name) as first_name)
                    ])
                );
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_with_multiple_placeholder_boolean_operators_and_wildcard_expansion() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    salary (EQL),
                    age (EQL),
                }
            }
        });

        let statement = parse("select * from users WHERE salary > $1 AND age <= $2");

        match type_check(schema, &statement) {
            Ok(typed) => {
                let a = Value::Eql(EqlValue(TableColumn {
                    table: id("users"),
                    column: id("salary"),
                }));

                let b = Value::Eql(EqlValue(TableColumn {
                    table: id("users"),
                    column: id("age"),
                }));

                assert_eq!(typed.params, vec![a, b]);

                assert_eq!(
                    typed.projection,
                    Some(projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.salary) as salary),
                        (EQL(users.age) as age)
                    ])
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
                    salary (EQL),
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn window_function() {
        let _ = tracing_subscriber::fmt::try_init();

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
                    rank() over (partition by department_name order by salary desc)
                from
                   employees
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn window_function_with_forward_reference() {
        let _ = tracing_subscriber::fmt::try_init();

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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn common_table_expressions() {
        let _ = tracing_subscriber::fmt::try_init();

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
                // eprintln!("Error: {}", err, err.source());
                panic!("type check failed: {:#?}", err)
            }
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn aggregates() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    department,
                    age,
                    salary (EQL),
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.age) as max),
                (EQL(employees.salary) as min)
            ])
        );
    }

    #[test]
    fn insert() {
        let _ = tracing_subscriber::fmt::try_init();

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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.projection, Some(Projection::Empty));
    }

    #[test]
    fn insert_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn update() {
        let _ = tracing_subscriber::fmt::try_init();

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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.projection, Some(Projection::Empty));
    }

    #[test]
    fn update_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn delete() {
        let _ = tracing_subscriber::fmt::try_init();

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
                delete from employees where salary > 200000
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.projection, Some(Projection::Empty));
    }

    #[test]
    fn delete_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

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
                delete from employees where salary > 200000 returning *
            "#,
        );

        let typed = match type_check(schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn select_with_literal_subsitution() {
        let _ = tracing_subscriber::fmt::try_init();

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
                select * from employees where salary > 200000
            "#,
        );

        let typed = match type_check(schema.clone(), &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert!(typed.literals.contains(&(
            EqlValue(TableColumn {
                table: id("employees"),
                column: id("salary")
            }),
            &ast::Expr::Value(ast::Value::Number(200000.into(), false))
        )));

        let transformed_statement = match typed.transform(HashMap::from_iter([(
            &ast::Expr::Value(ast::Value::Number(200000.into(), false)),
            ast::Expr::Value(ast::Value::SingleQuotedString("ENCRYPTED".into())),
        )])) {
            Ok(transformed_statement) => transformed_statement,
            Err(err) => panic!("statement transformation failed: {}", err),
        };

        // This type checks the transformed statement so we can get hold of the encrypted literal.
        let typed = match type_check(schema, &transformed_statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert!(typed.literals.contains(&(
            EqlValue(TableColumn {
                table: id("employees"),
                column: id("salary")
            }),
            &ast::Expr::Value(ast::Value::SingleQuotedString("ENCRYPTED".into())),
        )));
    }

    #[test]
    fn pathologically_complex_sql_statement() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = resolver(schema! {
            tables: {
                employees: {
                    id,
                    department_id,
                    name,
                    salary (EQL),
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.projection,
            Some(projection![
                (EQL(employees.salary) as min_salary),
                (EQL(employees.salary) as y),
                (NATIVE(employees.id) as id),
                (NATIVE(employees.department_id) as department_id),
                (NATIVE(employees.name) as name),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn literals_or_param_placeholders_in_outermost_projection() {
        let _ = tracing_subscriber::fmt::try_init();

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
            type_check(schema.clone(), statement)
                .unwrap()
                .projection
                .as_ref()
                .map(ignore_aliases)
        };

        assert_transitive_eq(&[
            projection_type(&parse("select 1")),
            projection_type(&parse("select t from (select 1 as t)")),
            projection_type(&parse("select * from (select 1)")),
            projection_type(&parse("select $1")),
            projection_type(&parse("select t from (select $1 as t)")),
            projection_type(&parse("select * from (select $1)")),
            Some(projection![(NATIVE)]),
        ]);
    }

    #[test]
    fn update_with_concat_regression() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = resolver(schema! {
            tables: {
                example: {
                    encrypted_str (EQL),
                    other_str,
                }
            }
        });

        // Can't use concat in an update on an EQL column
        let statement = parse("update example set encrypted_str = encrypted_str || 'a'");

        let err = type_check(schema.clone(), &statement)
            .expect_err("expected type check to fail, but it succeeded");

        assert_eq!(err.to_string(), "type Constructor(Value(EQL(example.encrypted_str))) cannot be unified with Constructor(Value(Native))");

        // Can use concat in an update on a plaintext column
        let statement = parse("update example set other_str = other_str || 'a'");
        type_check(schema, &statement).expect("expected type check to succeed, but it failed");
    }
}
