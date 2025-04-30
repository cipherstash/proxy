//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod dep;
mod display_helpers;
mod eql_mapper;
mod importer;
mod inference;
mod iterator_ext;
mod model;
mod param;
mod scope_tracker;
mod transformation_rules;
mod type_checked_statement;

#[cfg(test)]
mod test_helpers;

pub use display_helpers::*;
pub use eql_mapper::*;
pub use model::*;
pub use param::*;
pub use type_checked_statement::*;
pub use unifier::{EqlValue, NativeValue, TableColumn};

pub(crate) use dep::*;
pub(crate) use inference::*;
pub(crate) use scope_tracker::*;
pub(crate) use transformation_rules::*;

#[cfg(test)]
mod test {
    use super::test_helpers::*;
    use super::type_check;
    use crate::col;
    use crate::projection;
    use crate::test_helpers;
    use crate::Param;
    use crate::Schema;
    use crate::TableResolver;
    use crate::{
        schema, unifier::EqlValue, unifier::NativeValue, Projection, ProjectionColumn, TableColumn,
        Value,
    };
    use pretty_assertions::assert_eq;
    use sqltk::parser::ast::Statement;
    use sqltk::parser::ast::{self as ast};
    use sqltk::AsNodeKey;
    use std::collections::HashMap;
    use std::sync::Arc;
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
                    projection![(NATIVE(users.email) as email)]
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
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("select email from users WHERE email = 'hello@cipherstash.com'");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert_eq!(typed.projection, projection![(EQL(users.email) as email)]);

                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into())
                )));
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
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users (id, email) VALUES (42, 'hello@cipherstash.com')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into())
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
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("INSERT INTO users VALUES (42, 'hello@cipherstash.com', 'James')");

        match type_check(schema, &statement) {
            Ok(typed) => {
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into())
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
                assert!(typed.literals.contains(&(
                    EqlValue(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }),
                    &ast::Value::SingleQuotedString("hello@cipherstash.com".into())
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

                let (_, param_value) = typed.params.first().unwrap();

                assert_eq!(param_value, &v);

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
            Err(err) => panic!("type check failed: {:#?}", err),
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

                assert_eq!(typed.params, vec![(Param(1), a), (Param(2), b)]);

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

                assert_eq!(typed.params, vec![(Param(1), a), (Param(2), b)]);

                assert_eq!(
                    typed.projection,
                    projection![
                        (NATIVE(users.id) as id),
                        (EQL(users.salary) as salary),
                        (EQL(users.age) as age)
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
            projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (EQL(employees.salary) as salary)
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
                panic!("type check failed: {:#?}", err)
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
    fn aggregates() {
        // init_tracing();

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
            projection![
                (NATIVE(employees.age) as max),
                (EQL(employees.salary) as min)
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.projection, Projection::Empty);
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.projection, Projection::Empty);
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
            Err(err) => panic!("type check failed: {:#?}", err),
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

        assert_eq!(typed.projection, Projection::Empty);
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
    fn select_with_literal_subsitution() {
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
                select * from employees where salary > 200000
            "#,
        );

        let typed = match type_check(schema.clone(), &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.literals,
            vec![(
                EqlValue(TableColumn {
                    table: id("employees"),
                    column: id("salary")
                }),
                &ast::Value::Number(200000.into(), false)
            )]
        );

        let transformed_statement = match typed.transform(HashMap::from_iter([(
            typed.literals[0].1.as_node_key(),
            ast::Value::SingleQuotedString("ENCRYPTED".into()),
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
            &ast::Value::SingleQuotedString("ENCRYPTED".into()),
        )));
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
            projection![
                (EQL(employees.salary) as min_salary),
                (EQL(employees.salary) as y),
                (NATIVE(employees.id) as id),
                (NATIVE(employees.department_id) as department_id),
                (NATIVE(employees.name) as name),
                (EQL(employees.salary) as salary)
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
            Projection::WithColumns(vec![ProjectionColumn {
                alias: None,
                ty: Value::Native(NativeValue(None)),
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
    fn group_by_eql_column() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                    first_name,
                }
            }
        });

        let statement = parse("SELECT email FROM users GROUP BY email");

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(HashMap::new()) {
                    Ok(statement) => assert_eq!(
                        statement.to_string(),
                        "SELECT CS_GROUPED_VALUE_V1(email) AS email FROM users GROUP BY CS_ORE_64_8_V1(email)".to_string()
                    ),
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn modify_aggregate_when_eql_column_affected_by_group_by_of_other_column() {
        // init_tracing();
        let schema = resolver(schema! {
            tables: {
                employees: {
                    id (PK),
                    department,
                    salary (EQL),
                }
            }
        });

        let statement =
            parse("SELECT MIN(salary), MAX(salary), department FROM employees GROUP BY department");

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(HashMap::new()) {
                    Ok(statement) => assert_eq!(
                        statement.to_string(),
                        "SELECT CS_MIN_V1(salary) AS MIN, CS_MAX_V1(salary) AS MAX, department FROM employees GROUP BY department".to_string()
                    ),
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
        test_jsonb_operator("->");
    }

    #[test]
    fn jsonb_operator_long_arrow() {
        test_jsonb_operator("->>");
    }

    #[test]
    fn jsonb_operator_hash_arrow() {
        test_jsonb_operator("#>");
    }

    #[test]
    fn jsonb_operator_hash_long_arrow() {
        test_jsonb_operator("#>>");
    }

    #[test]
    fn jsonb_operator_hash_at_at() {
        test_jsonb_operator("@@");
    }

    #[test]
    fn jsonb_operator_at_question() {
        test_jsonb_operator("@?");
    }

    #[test]
    fn jsonb_operator_question() {
        test_jsonb_operator("?");
    }

    #[test]
    fn jsonb_operator_question_and() {
        test_jsonb_operator("?&");
    }

    #[test]
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

    fn test_jsonb_operator(op: &'static str) {
        let schema = resolver(schema! {
            tables: {
                patients: {
                    id (PK),
                    notes (EQL),
                }
            }
        });

        let statement = parse(&format!("SELECT id, notes {} 'medications' AS meds FROM patients", op));

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(test_helpers::dummy_encrypted_json_selector(&typed, "medications")) {
                    Ok(statement) => assert_eq!(
                        statement.to_string(),
                        format!("SELECT id, notes {} '<encrypted-selector(medications)>' AS meds FROM patients", op)
                    ),
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }
}
