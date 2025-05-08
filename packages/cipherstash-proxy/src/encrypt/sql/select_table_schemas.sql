SELECT
    t.table_schema,
    t.table_name,
    array_agg(distinct k.column_name)::text[] AS primary_keys,
    array_agg(c.column_name)::text[] AS columns,
    array_agg(c.udt_name)::text[] AS column_type_names
FROM
    information_schema.tables t
LEFT JOIN
    information_schema.table_constraints tc ON tc.table_schema = t.table_schema
                                             AND tc.table_name = t.table_name
                                             AND tc.constraint_type = 'PRIMARY KEY'
LEFT JOIN
    information_schema.key_column_usage k ON k.table_schema = tc.table_schema
                                            AND k.table_name = tc.table_name
                                            AND k.constraint_name = tc.constraint_name
LEFT JOIN
    information_schema.columns c ON c.table_schema = t.table_schema
                                            AND c.table_name = t.table_name
WHERE
    t.table_type = 'BASE TABLE'
GROUP BY
    t.table_schema, t.table_name
ORDER BY
    t.table_schema, t.table_name;



