-- EQL v3 integration-test schema.
--
-- Encrypted columns are self-configuring domain types (`eql_v3_<token>_<cap>`),
-- so there is no `eql_v2.add_search_config`, `add_encrypted_constraint`, or
-- `eql_v2_configuration` table — the domain type declares the encryption and
-- its capabilities, and the proxy infers its config from the schema.
--
-- Capability -> domain suffix:
--   equality only            -> _eq        (HMAC)
--   ordering (default, CLLW-OPE) -> _ord    (op; also supports equality)
--   ordering (block-ORE)     -> _ord_ore   (ob)
--   text search (eq+ord+match)   -> _search / _search_ore
--   fuzzy match              -> _match     (bloom)
--   encrypted JSON (SteVec)  -> json_search
-- `boolean` is storage-only in v3 (a two-value column leaks its distribution
-- under any index), so encrypted bool columns carry no searchable capability.

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


-- Exciting cipherstash table. Scalars use the default CLLW-OPE ordering domain
-- (`_ord`, which also supports equality); text is fully searchable
-- (`text_search` = eq + ord + match); bool is storage-only.
DROP TABLE IF EXISTS encrypted;
CREATE TABLE encrypted (
    id bigint,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v3_text_search,
    encrypted_bool eql_v3_boolean,
    encrypted_int2 eql_v3_smallint_ord,
    encrypted_int4 eql_v3_integer_ord,
    encrypted_int8 eql_v3_bigint_ord,
    encrypted_float8 eql_v3_double_ord,
    encrypted_date eql_v3_date_ord,
    encrypted_jsonb eql_v3_json_search,
    encrypted_jsonb_filtered eql_v3_json_search,
    PRIMARY KEY(id)
);

-- A storage-only encrypted column (encrypt/decrypt, no searchable capability).
DROP TABLE IF EXISTS unconfigured;
CREATE TABLE unconfigured (
    id bigint,
    encrypted_unconfigured eql_v3_text,
    PRIMARY KEY(id)
);


-- Per-test encrypted index fixture tables.
--
-- Each integration test that exercises ORE/OPE range or order operators gets
-- its own table to avoid parallel-test races on a shared table. `kind` selects
-- the ordering domain family: `ore` -> block-ORE (`_ord_ore`, `text_search_ore`);
-- `ope` -> CLLW-OPE (`_ord_ope`, `text_ord_ope`). Boolean is storage-only in v3,
-- so the fixtures no longer carry an encrypted_bool column.
DO $$
DECLARE
  spec record;
  tn text;
  text_domain text;
  ord_suffix text;
BEGIN
  FOR spec IN
    -- map_ore_index_where (one per column type) + map_ore_index_order (one per test fn)
    SELECT 'ore'::text AS kind, unnest(ARRAY[
      'encrypted_ore_where_int2',
      'encrypted_ore_where_int4',
      'encrypted_ore_where_int8',
      'encrypted_ore_where_float8',
      'encrypted_ore_where_date',
      'encrypted_ore_where_text',
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
    ]) AS table_name
    UNION ALL
    -- map_ope_index_where (one per column type) + map_ope_index_order (one per test fn)
    SELECT 'ope'::text AS kind, unnest(ARRAY[
      'encrypted_ope_where_int2',
      'encrypted_ope_where_int4',
      'encrypted_ope_where_int8',
      'encrypted_ope_where_float8',
      'encrypted_ope_where_date',
      'encrypted_ope_where_text',
      'encrypted_ope_order_text_asc',
      'encrypted_ope_order_text_desc',
      'encrypted_ope_order_int4_asc',
      'encrypted_ope_order_int4_desc',
      'encrypted_ope_order_nulls_last',
      'encrypted_ope_order_nulls_first'
    ]) AS table_name
  LOOP
    tn := spec.table_name;

    IF spec.kind = 'ore' THEN
      text_domain := 'eql_v3_text_search_ore';
      ord_suffix := '_ord_ore';
    ELSE
      text_domain := 'eql_v3_text_ord_ope';
      ord_suffix := '_ord_ope';
    END IF;

    EXECUTE format('DROP TABLE IF EXISTS %I CASCADE', tn);
    EXECUTE format(
      'CREATE TABLE %I (
        id bigint,
        plaintext text,
        plaintext_date date,
        encrypted_text %s,
        encrypted_int2 eql_v3_smallint%s,
        encrypted_int4 eql_v3_integer%s,
        encrypted_int8 eql_v3_bigint%s,
        encrypted_float8 eql_v3_double%s,
        encrypted_date eql_v3_date%s,
        PRIMARY KEY(id)
      )', tn, text_domain, ord_suffix, ord_suffix, ord_suffix, ord_suffix, ord_suffix);
  END LOOP;
END $$;


-- This is the exact same schema as `encrypted` but using a database-generated
-- primary key. It is required to remove flake from the Elixir integration test
-- suite.
-- TODO: port all the rest of our integration tests to this schema.
DROP TABLE IF EXISTS encrypted_elixir;
CREATE TABLE encrypted_elixir (
    id serial,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v3_text_search,
    encrypted_bool eql_v3_boolean,
    encrypted_int2 eql_v3_smallint_ord,
    encrypted_int4 eql_v3_integer_ord,
    encrypted_int8 eql_v3_bigint_ord,
    encrypted_float8 eql_v3_double_ord,
    encrypted_date eql_v3_date_ord,
    encrypted_jsonb eql_v3_json_search,
    encrypted_jsonb_filtered eql_v3_json_search,
    PRIMARY KEY(id)
);

DROP TABLE IF EXISTS unconfigured_elixir;
CREATE TABLE unconfigured_elixir (
    id serial,
    encrypted_unconfigured eql_v3_text,
    PRIMARY KEY(id)
);
