
TRUNCATE TABLE public.eql_v2_configuration;

-- Regular old table
DROP TABLE IF EXISTS plaintext;
CREATE TABLE plaintext (
    id bigint,
    plaintext text,
    PRIMARY KEY(id)
);

DO $$
  BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'domain_type_with_check') THEN
      CREATE DOMAIN domain_type_with_check AS VARCHAR(2) CHECK (VALUE ~ '^[A-Z]{2}$');
    END IF;
  END
$$;


-- Exciting cipherstash table
DROP TABLE IF EXISTS encrypted;
CREATE TABLE encrypted (
    id bigint,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v2_encrypted,
    encrypted_bool eql_v2_encrypted,
    encrypted_int2 eql_v2_encrypted,
    encrypted_int4 eql_v2_encrypted,
    encrypted_int8 eql_v2_encrypted,
    encrypted_float8 eql_v2_encrypted,
    encrypted_date eql_v2_encrypted,
    encrypted_jsonb eql_v2_encrypted,
    encrypted_jsonb_filtered eql_v2_encrypted,
    PRIMARY KEY(id)
);

DROP TABLE IF EXISTS unconfigured;
CREATE TABLE unconfigured (
    id bigint,
    encrypted_unconfigured eql_v2_encrypted,
    PRIMARY KEY(id)
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_text',
  'unique',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_text',
  'match',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_text',
  'ore',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_bool',
  'unique',
  'boolean'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_bool',
  'ore',
  'boolean'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int2',
  'unique',
  'small_int'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int2',
  'ore',
  'small_int'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int4',
  'unique',
  'int'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int4',
  'ore',
  'int'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int8',
  'unique',
  'big_int'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_int8',
  'ore',
  'big_int'
);


SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_float8',
  'unique',
  'double'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_float8',
  'ore',
  'double'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_date',
  'unique',
  'date'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_date',
  'ore',
  'date'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "encrypted/encrypted_jsonb"}'
);

SELECT eql_v2.add_search_config(
  'encrypted',
  'encrypted_jsonb_filtered',
  'ste_vec',
  'jsonb',
  '{"prefix": "encrypted/encrypted_jsonb_filtered", "term_filters": [{"kind": "downcase"}]}'
);

SELECT eql_v2.add_encrypted_constraint('encrypted', 'encrypted_text');


-- Per-test ORE-indexed tables.
-- Each integration test that exercises ORE range/order operators gets its own
-- table. Eliminates parallel-test races on a shared `encrypted` table without
-- having to mark tests `#[serial]`.
--
-- Schema mirrors `encrypted` minus the jsonb columns (these ORE tests never
-- touch jsonb). Each table gets the same `add_search_config` and constraint
-- calls as the original `encrypted` table.
DO $$
DECLARE
  test_tables text[] := ARRAY[
    -- map_ore_index_where (one per column type)
    'encrypted_ore_where_int2',
    'encrypted_ore_where_int4',
    'encrypted_ore_where_int8',
    'encrypted_ore_where_float8',
    'encrypted_ore_where_date',
    'encrypted_ore_where_text',
    'encrypted_ore_where_bool',
    -- map_ore_index_order (one per test fn)
    'encrypted_ore_order_text',
    'encrypted_ore_order_text_desc',
    'encrypted_ore_order_nulls_last',
    'encrypted_ore_order_nulls_first',
    'encrypted_ore_order_qualified',
    'encrypted_ore_order_qualified_alias',
    'encrypted_ore_order_no_select_projection',
    'encrypted_ore_order_plaintext_column',
    'encrypted_ore_order_plaintext_and_eql',
    'encrypted_ore_order_simple_protocol',
    'encrypted_ore_order_int2',
    'encrypted_ore_order_int2_desc',
    'encrypted_ore_order_int4',
    'encrypted_ore_order_int4_desc',
    'encrypted_ore_order_int8',
    'encrypted_ore_order_int8_desc',
    'encrypted_ore_order_float8',
    'encrypted_ore_order_float8_desc'
  ];
  tn text;
BEGIN
  FOREACH tn IN ARRAY test_tables LOOP
    EXECUTE format('DROP TABLE IF EXISTS %I CASCADE', tn);
    EXECUTE format(
      'CREATE TABLE %I (
        id bigint,
        plaintext text,
        plaintext_date date,
        encrypted_text eql_v2_encrypted,
        encrypted_bool eql_v2_encrypted,
        encrypted_int2 eql_v2_encrypted,
        encrypted_int4 eql_v2_encrypted,
        encrypted_int8 eql_v2_encrypted,
        encrypted_float8 eql_v2_encrypted,
        encrypted_date eql_v2_encrypted,
        PRIMARY KEY(id)
      )', tn);

    PERFORM eql_v2.add_search_config(tn, 'encrypted_text', 'unique', 'text');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_text', 'match', 'text');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_text', 'ore', 'text');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_bool', 'unique', 'boolean');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_bool', 'ore', 'boolean');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int2', 'unique', 'small_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int2', 'ore', 'small_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int4', 'unique', 'int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int4', 'ore', 'int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int8', 'unique', 'big_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int8', 'ore', 'big_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_float8', 'unique', 'double');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_float8', 'ore', 'double');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_date', 'unique', 'date');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_date', 'ore', 'date');

    PERFORM eql_v2.add_encrypted_constraint(tn, 'encrypted_text');
  END LOOP;
END $$;


-- Per-test OPE-indexed tables (parallels the ORE block above; uses 'ope' index).
DO $$
DECLARE
  test_tables text[] := ARRAY[
    -- map_ope_index_where (one per column type)
    'encrypted_ope_where_int2',
    'encrypted_ope_where_int4',
    'encrypted_ope_where_int8',
    'encrypted_ope_where_float8',
    'encrypted_ope_where_date',
    'encrypted_ope_where_text',
    'encrypted_ope_where_bool',
    -- map_ope_index_order (one per test fn)
    'encrypted_ope_order_text_asc',
    'encrypted_ope_order_text_desc',
    'encrypted_ope_order_int4_asc',
    'encrypted_ope_order_int4_desc',
    'encrypted_ope_order_nulls_last',
    'encrypted_ope_order_nulls_first'
  ];
  tn text;
BEGIN
  FOREACH tn IN ARRAY test_tables LOOP
    EXECUTE format('DROP TABLE IF EXISTS %I CASCADE', tn);
    EXECUTE format(
      'CREATE TABLE %I (
        id bigint,
        plaintext text,
        plaintext_date date,
        encrypted_text eql_v2_encrypted,
        encrypted_bool eql_v2_encrypted,
        encrypted_int2 eql_v2_encrypted,
        encrypted_int4 eql_v2_encrypted,
        encrypted_int8 eql_v2_encrypted,
        encrypted_float8 eql_v2_encrypted,
        encrypted_date eql_v2_encrypted,
        PRIMARY KEY(id)
      )', tn);

    PERFORM eql_v2.add_search_config(tn, 'encrypted_text', 'unique', 'text');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_text', 'ope', 'text');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_bool', 'unique', 'boolean');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_bool', 'ope', 'boolean');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int2', 'unique', 'small_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int2', 'ope', 'small_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int4', 'unique', 'int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int4', 'ope', 'int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int8', 'unique', 'big_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_int8', 'ope', 'big_int');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_float8', 'unique', 'double');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_float8', 'ope', 'double');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_date', 'unique', 'date');
    PERFORM eql_v2.add_search_config(tn, 'encrypted_date', 'ope', 'date');

    PERFORM eql_v2.add_encrypted_constraint(tn, 'encrypted_text');
  END LOOP;
END $$;


-- This is the exact same schema as above but using a database-generated primary key.
-- It is required to remove flake form the Elixir integration test suite.
-- TODO: port all the rest of our integration tests to this schema.
DROP TABLE IF EXISTS encrypted_elixir;
CREATE TABLE encrypted_elixir (
    id serial,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v2_encrypted,
    encrypted_bool eql_v2_encrypted,
    encrypted_int2 eql_v2_encrypted,
    encrypted_int4 eql_v2_encrypted,
    encrypted_int8 eql_v2_encrypted,
    encrypted_float8 eql_v2_encrypted,
    encrypted_date eql_v2_encrypted,
    encrypted_jsonb eql_v2_encrypted,
    encrypted_jsonb_filtered eql_v2_encrypted,
    PRIMARY KEY(id)
);

DROP TABLE IF EXISTS unconfigured_elixir;
CREATE TABLE unconfigured_elixir (
    id serial,
    encrypted_unconfigured eql_v2_encrypted,
    PRIMARY KEY(id)
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_text',
  'unique',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_text',
  'match',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_text',
  'ore',
  'text'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_bool',
  'unique',
  'boolean'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_bool',
  'ore',
  'boolean'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int2',
  'unique',
  'small_int'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int2',
  'ore',
  'small_int'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int4',
  'unique',
  'int'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int4',
  'ore',
  'int'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int8',
  'unique',
  'big_int'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_int8',
  'ore',
  'big_int'
);


SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_float8',
  'unique',
  'double'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_float8',
  'ore',
  'double'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_date',
  'unique',
  'date'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_date',
  'ore',
  'date'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "encrypted/encrypted_jsonb"}'
);

SELECT eql_v2.add_search_config(
  'encrypted_elixir',
  'encrypted_jsonb_filtered',
  'ste_vec',
  'jsonb',
  '{"prefix": "encrypted/encrypted_jsonb_filtered", "term_filters": [{"kind": "downcase"}]}'
);

SELECT eql_v2.add_encrypted_constraint('encrypted_elixir', 'encrypted_text');

