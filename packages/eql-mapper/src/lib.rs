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
pub use unifier::{EqlTerm, EqlTrait, EqlTraits, EqlValue, NativeValue, TableColumn};

pub(crate) use dep::*;
pub(crate) use inference::*;
pub(crate) use scope_tracker::*;
pub(crate) use transformation_rules::*;

#[cfg(test)]
mod test {
    use super::{test_helpers::*, type_check};
    use crate::{
        projection, schema, test_helpers,
        unifier::{EqlTerm, EqlTrait, EqlTraits, EqlValue, NativeValue},
        Param, Projection, ProjectionColumn, Schema, TableColumn, TableResolver, Value,
    };
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
                    projection![(EQL(users.email: Eq) as email)]
                );

                eprintln!("TYPED LITS: {:#?}", typed.literals);

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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("statement transformation failed: {}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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
            Err(err) => panic!("statement transformation failed: {}", err),
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
            Err(err) => panic!("type check failed: {:#?}", err),
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

        let statement = parse("
            SELECT jsonb_path_query(eql_col, '$.secret'), jsonb_path_query(native_col, '$.not-secret') FROM employees
        ");

        match type_check(schema, &statement) {
            Ok(typed) => {
                match typed.transform(test_helpers::dummy_encrypted_json_selector(
                    &statement,
                    ast::Value::SingleQuotedString("$.secret".into()),
                )) {
                    Ok(statement) => {
                        assert_eq!(
                            statement.to_string(),
                            "SELECT eql_v2.jsonb_path_query(eql_col, '<encrypted-selector($.secret)>'::JSONB::eql_v2_encrypted), jsonb_path_query(native_col, '$.not-secret') FROM employees"
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
            "SELECT id, {}({}) AS meds FROM patients",
            fn_name, args_in
        ));

        let args_encrypted = args
            .iter()
            .map(|expr| match expr {
                ast::Expr::Identifier(ident) => ident.to_string(),
                ast::Expr::Value(ast::ValueWithSpan {
                    value: ast::Value::SingleQuotedString(s),
                    span: _,
                }) => {
                    format!("'<encrypted-selector({})>'::JSONB::eql_v2_encrypted", s)
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
                    value.clone(),
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
                match typed.transform(test_helpers::dummy_encrypted_json_selector(&statement, ast::Value::SingleQuotedString("medications".to_owned()))) {
                    Ok(statement) => assert_eq!(
                        statement.to_string(),
                        format!("SELECT id, notes {op} '<encrypted-selector(medications)>'::JSONB::eql_v2_encrypted AS meds FROM patients")
                    ),
                    Err(err) => panic!("transformation failed: {err}"),
                }
            }
            Err(err) => panic!("type check failed: {err}"),
        }
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
}
