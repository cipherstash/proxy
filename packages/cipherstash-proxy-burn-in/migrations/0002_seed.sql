-- Copied from pg-proto's burn-in seed-v1 fixture.
DO $fixture_seed$
BEGIN
    TRUNCATE burnin_type_lab.samples, burnin_type_lab.bulk_values,
        burnin_commerce.order_lines, burnin_commerce.orders,
        burnin_commerce.products, burnin_commerce.customers;

    INSERT INTO burnin_type_lab.samples
        (id, scalar, nullable_text, binary_value, tags, document, wide_text)
    VALUES
        (1, 10, NULL, decode('000102ff', 'hex'), ARRAY['alpha', 'one'],
            '{"kind":"alpha","enabled":true}'::jsonb, repeat('wide-alpha-', 40)),
        (2, 20, 'second', decode('10203040', 'hex'), ARRAY[]::text[],
            '{"kind":"beta","count":2}'::jsonb, repeat('wide-beta-', 40)),
        (3, 30, NULL, decode('deadbeef', 'hex'), ARRAY['nullable'],
            '{"kind":"gamma","values":[1,2,3]}'::jsonb, repeat('wide-gamma-', 40)),
        (4, 40, 'fourth', decode('cafebabe', 'hex'), ARRAY['delta', 'four'],
            '{"kind":"delta","value":null}'::jsonb, repeat('wide-delta-', 40));

    INSERT INTO burnin_type_lab.bulk_values (id, nullable_text, binary_value, wide_text)
    SELECT value,
        CASE WHEN value % 5 = 0 THEN NULL ELSE 'value-' || value END,
        decode(md5(value::text), 'hex'),
        repeat(lpad(value::text, 8, '0') || '-deterministic-wide-value-', 12)
    FROM generate_series(1, 4096) AS value;

    INSERT INTO burnin_commerce.customers (id, name)
    SELECT value, 'customer-' || lpad(value::text, 3, '0')
    FROM generate_series(1, 64) AS value;
    INSERT INTO burnin_commerce.products (id, sku, price_cents)
    SELECT value, 'SKU-' || lpad(value::text, 4, '0'), 100 + value * 7
    FROM generate_series(1, 128) AS value;
    INSERT INTO burnin_commerce.orders (id, customer_id, status)
    SELECT value, ((value - 1) % 64) + 1,
        CASE WHEN value % 4 = 0 THEN 'shipped' ELSE 'open' END
    FROM generate_series(1, 256) AS value;
    INSERT INTO burnin_commerce.order_lines (order_id, line_number, product_id, quantity)
    SELECT order_id, line_number, ((order_id * 3 + line_number - 1) % 128) + 1,
        ((order_id + line_number) % 5) + 1
    FROM generate_series(1, 256) AS order_id
    CROSS JOIN generate_series(1, 2) AS line_number;
END
$fixture_seed$;

