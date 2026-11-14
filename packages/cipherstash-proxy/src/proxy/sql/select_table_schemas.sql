SELECT
    t.table_schema,
    t.table_schema::text = ANY (current_schemas(false)::text[]) AS on_search_path,
    t.table_name,
    array_agg(c.column_name)::text[] AS columns,
    array_agg(c.udt_name)::text[] AS column_type_names,
    -- EQL v3 encrypted columns are jsonb-backed DOMAINs, so `udt_name` reports
    -- the base type (`jsonb`); the domain typname (e.g. `eql_v3_integer_ord`)
    -- is only available via `domain_name`. NULL for non-domain columns.
    array_agg(c.domain_name)::text[] AS column_domain_names
FROM
    information_schema.tables t
LEFT JOIN
    information_schema.table_constraints tc ON tc.table_schema = t.table_schema
                                             AND tc.table_name = t.table_name
                                             AND tc.constraint_type = 'PRIMARY KEY'
LEFT JOIN
    information_schema.columns c ON c.table_schema = t.table_schema
                                            AND c.table_name = t.table_name
WHERE
    t.table_type = 'BASE TABLE'
    -- Every schema but the system ones (`pg_` is reserved for them).
    --
    -- Only tables `on_search_path`, the schemas this connection resolves
    -- unqualified names in, are loaded. The schema map records each table's
    -- schema, but the encrypt config is keyed on the bare table name, so a
    -- same-named table in another schema (another tenant's, a staging copy, a
    -- leftover eql_v2) would share or conflict with the served table's column
    -- config. `false` excludes the implicitly-searched pg_catalog, leaving
    -- exactly the schemas an unqualified `users` could mean.
    --
    -- Tables in other schemas only say whether the database has encrypted
    -- columns at all, so that a write to a table Proxy cannot resolve fails
    -- closed even when every encrypted table is off the search path.
    AND t.table_schema::text NOT LIKE 'pg\_%'
    AND t.table_schema::text <> 'information_schema'
GROUP BY
    t.table_schema, t.table_name
ORDER BY
    -- Search-path order, so rows for a name come in the order PostgreSQL
    -- would resolve them; tables off the search path come last. Consumers keep every row: a bare name found in two
    -- schemas stays ambiguous, and qualified names resolve to their schema.
    array_position(current_schemas(false)::text[], t.table_schema::text),
    t.table_name;

