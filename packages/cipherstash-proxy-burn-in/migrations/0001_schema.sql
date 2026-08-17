-- Copied from pg-proto's burn-in schema-v1 fixture.
DO $fixture_schema$
BEGIN
    CREATE SCHEMA IF NOT EXISTS burnin_type_lab;
    CREATE TABLE IF NOT EXISTS burnin_type_lab.samples (
        id integer PRIMARY KEY,
        scalar integer NOT NULL,
        nullable_text text,
        binary_value bytea NOT NULL,
        tags text[] NOT NULL,
        document jsonb NOT NULL,
        wide_text text NOT NULL
    );
    CREATE TABLE IF NOT EXISTS burnin_type_lab.bulk_values (
        id integer PRIMARY KEY,
        nullable_text text,
        binary_value bytea NOT NULL,
        wide_text text NOT NULL
    );

    CREATE SCHEMA IF NOT EXISTS burnin_commerce;
    CREATE TABLE IF NOT EXISTS burnin_commerce.customers (
        id integer PRIMARY KEY,
        name text NOT NULL
    );
    CREATE TABLE IF NOT EXISTS burnin_commerce.products (
        id integer PRIMARY KEY,
        sku text NOT NULL UNIQUE,
        price_cents integer NOT NULL CHECK (price_cents > 0)
    );
    CREATE TABLE IF NOT EXISTS burnin_commerce.orders (
        id integer PRIMARY KEY,
        customer_id integer NOT NULL REFERENCES burnin_commerce.customers(id),
        status text NOT NULL
    );
    CREATE TABLE IF NOT EXISTS burnin_commerce.order_lines (
        order_id integer NOT NULL REFERENCES burnin_commerce.orders(id),
        line_number integer NOT NULL,
        product_id integer NOT NULL REFERENCES burnin_commerce.products(id),
        quantity integer NOT NULL CHECK (quantity > 0),
        PRIMARY KEY (order_id, line_number)
    );
END
$fixture_schema$;

