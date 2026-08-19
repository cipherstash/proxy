-- EQL is installed from cipherstash-encrypt.sql before this migration runs.
-- Keep these tables in public: Proxy loads only schemas on its search path and
-- EQL Mapper resolves tables in a single, unqualified namespace.
DROP SCHEMA IF EXISTS burnin_type_lab CASCADE;
DROP SCHEMA IF EXISTS burnin_commerce CASCADE;

DROP TABLE IF EXISTS public.burnin_commerce_order_lines;
DROP TABLE IF EXISTS public.burnin_commerce_orders;
DROP TABLE IF EXISTS public.burnin_commerce_products;
DROP TABLE IF EXISTS public.burnin_commerce_customers;
DROP TABLE IF EXISTS public.burnin_type_lab_bulk_values;
DROP TABLE IF EXISTS public.burnin_type_lab_samples;

CREATE TABLE public.burnin_type_lab_samples (
    id integer PRIMARY KEY,
    scalar eql_v3_integer_ord NOT NULL,
    nullable_text eql_v3_text,
    binary_value bytea NOT NULL,
    tags text[] NOT NULL,
    document eql_v3_json NOT NULL,
    wide_text eql_v3_text NOT NULL
);

CREATE TABLE public.burnin_type_lab_bulk_values (
    id integer PRIMARY KEY,
    nullable_text eql_v3_text,
    binary_value bytea NOT NULL,
    wide_text eql_v3_text NOT NULL
);

CREATE TABLE public.burnin_commerce_customers (
    id integer PRIMARY KEY,
    name eql_v3_text NOT NULL
);

CREATE TABLE public.burnin_commerce_products (
    id integer PRIMARY KEY,
    sku eql_v3_text NOT NULL,
    price_cents integer NOT NULL CHECK (price_cents > 0)
);

CREATE TABLE public.burnin_commerce_orders (
    id integer PRIMARY KEY,
    customer_id integer NOT NULL REFERENCES public.burnin_commerce_customers(id),
    status eql_v3_text NOT NULL
);

CREATE TABLE public.burnin_commerce_order_lines (
    order_id integer NOT NULL REFERENCES public.burnin_commerce_orders(id),
    line_number integer NOT NULL,
    product_id integer NOT NULL REFERENCES public.burnin_commerce_products(id),
    quantity integer NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (order_id, line_number)
);
