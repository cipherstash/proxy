-- EQL v3 port of tests/sql/schema.sql.
--
-- GATED: the proxy's eql-mapper cannot speak EQL v3 yet, so this fixture is
-- NOT applied by `postgres:setup` (which still applies the v2 schema.sql).
-- Apply it explicitly with `mise run postgres:setup:v3` after installing a
-- locally built EQL v3 (`CS_EQL_V3_PATH=... mise run postgres:eql:v3:setup`).
--
-- Key differences from the v2 fixture:
--
-- * There is no single `eql_v2_encrypted` type in v3. Each column uses a
--   per-scalar-per-capability jsonb domain (`eql_v3.<family>[_<capability>]`)
--   chosen from the column's v2 `eql_v2.add_search_config` calls:
--     - unique  -> `hm` term (`_eq`)
--     - ore     -> `ob` term (`_ord_ore`)
--     - ope     -> `op` term (`_ord_ope`)
--     - match   -> `bf` term (`_match`, text only)
--     - unique + match + ore (text) -> `_search`
--     - ste_vec (jsonb) -> `eql_v3.json`
--
-- * There are no `eql_v2.add_search_config` / `eql_v2.add_encrypted_constraint`
--   equivalents: v3 has no database-side configuration. Index/term
--   configuration lives client-side (the proxy's Encrypt config), and the
--   domains are fail-closed - their CHECK constraints reject payloads that
--   are missing the required terms, replacing `add_encrypted_constraint`.
--
-- * Ordering does not use an on-column operator class. It uses functional
--   btree indexes on term extractors, created where a test needs one, e.g.:
--     CREATE INDEX ON encrypted (eql_v3.ord_term(encrypted_text));
--     CREATE INDEX ON encrypted (eql_v3.ord_ope_term(encrypted_int4));
--   and GIN containment for SteVec documents:
--     CREATE INDEX ON encrypted USING GIN ((eql_v3.to_ste_vec_query(encrypted_jsonb)::jsonb) jsonb_path_ops);
--
-- This fixture reuses the v2 table names (`encrypted`, `unconfigured`, ...) so
-- the existing integration tests can ride on it unchanged once the mapper
-- speaks v3. Applying it therefore REPLACES the v2 fixture tables; re-run
-- `mise run postgres:setup` to restore the v2 fixture.

-- The v2 fixture truncates its configuration table here. v3 has no
-- database-side configuration table, but stale v2 configuration must not be
-- left pointing at tables whose columns are now v3 domains.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.tables
    WHERE table_schema = 'public' AND table_name = 'eql_v2_configuration'
  ) THEN
    TRUNCATE TABLE public.eql_v2_configuration;
  END IF;
END $$;

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


-- Exciting cipherstash table.
--
-- Column -> domain mapping (from the v2 add_search_config calls):
--   encrypted_text    unique + match + ore -> eql_v3.text_search (hm + ob + bf)
--   encrypted_bool    unique + ore         -> eql_v3.bool (storage-only: v3 bool
--                                             has no query capability domains)
--   encrypted_int2    unique + ore         -> eql_v3.int2_ord_ore
--   encrypted_int4    unique + ore         -> eql_v3.int4_ord_ore
--   encrypted_int8    unique + ore         -> eql_v3.int8_ord_ore
--   encrypted_float8  unique + ore         -> eql_v3.float8_ord_ore
--   encrypted_date    unique + ore         -> eql_v3.date_ord_ore
--   encrypted_jsonb   ste_vec              -> eql_v3.json
--   encrypted_jsonb_filtered ste_vec + term_filters -> eql_v3.json (term
--                                             filters are client-side in v3)
--
-- Note: the non-text `_ord_ore` domains only REQUIRE the `ob` term; the `hm`
-- term written for `unique` is carried as an additional payload field and
-- extracted with `eql_v3.eq_term` where equality is needed.
DROP TABLE IF EXISTS encrypted;
CREATE TABLE encrypted (
    id bigint,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v3.text_search,
    encrypted_bool eql_v3.bool,
    encrypted_int2 eql_v3.int2_ord_ore,
    encrypted_int4 eql_v3.int4_ord_ore,
    encrypted_int8 eql_v3.int8_ord_ore,
    encrypted_float8 eql_v3.float8_ord_ore,
    encrypted_date eql_v3.date_ord_ore,
    encrypted_jsonb eql_v3.json,
    encrypted_jsonb_filtered eql_v3.json,
    PRIMARY KEY(id)
);

-- "Unconfigured" in v3 means the proxy has no client-side Encrypt config for
-- the column. Database-side it is just a storage-only domain (no query terms
-- required); `eql_v3.text` is used as the representative storage envelope.
DROP TABLE IF EXISTS unconfigured;
CREATE TABLE unconfigured (
    id bigint,
    encrypted_unconfigured eql_v3.text,
    PRIMARY KEY(id)
);


-- Per-test encrypted index fixture tables.
--
-- Each integration test that exercises ORE/OPE range or order operators gets
-- its own table. This eliminates parallel-test races on a shared `encrypted`
-- table without having to mark tests `#[serial]`.
--
-- The schema mirrors `encrypted` minus the jsonb columns (these tests never
-- touch jsonb). `kind` is `ore` or `ope`, selecting the `_ord_ore` or
-- `_ord_ope` domain per column; ORE text columns additionally carry a match
-- (bf) term, so they use `eql_v3.text_search` while OPE text columns use
-- `eql_v3.text_ord_ope`. `encrypted_bool` is storage-only in v3 (see above).
DO $$
DECLARE
  spec record;
  tn text;
  text_domain text;
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
      'encrypted_ore_where_bool',
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
      'encrypted_ope_where_bool',
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
      text_domain := 'eql_v3.text_search';
    ELSE
      text_domain := 'eql_v3.text_ord_ope';
    END IF;

    EXECUTE format('DROP TABLE IF EXISTS %I CASCADE', tn);
    EXECUTE format(
      'CREATE TABLE %I (
        id bigint,
        plaintext text,
        plaintext_date date,
        encrypted_text %s,
        encrypted_bool eql_v3.bool,
        encrypted_int2 eql_v3.int2_ord_%s,
        encrypted_int4 eql_v3.int4_ord_%s,
        encrypted_int8 eql_v3.int8_ord_%s,
        encrypted_float8 eql_v3.float8_ord_%s,
        encrypted_date eql_v3.date_ord_%s,
        PRIMARY KEY(id)
      )', tn, text_domain, spec.kind, spec.kind, spec.kind, spec.kind, spec.kind);
  END LOOP;
END $$;


-- This is the exact same schema as above but using a database-generated primary key.
-- It is required to remove flake from the Elixir integration test suite.
-- TODO: port all the rest of our integration tests to this schema.
DROP TABLE IF EXISTS encrypted_elixir;
CREATE TABLE encrypted_elixir (
    id serial,
    plaintext text,
    plaintext_date date,
    plaintext_domain domain_type_with_check,
    encrypted_text eql_v3.text_search,
    encrypted_bool eql_v3.bool,
    encrypted_int2 eql_v3.int2_ord_ore,
    encrypted_int4 eql_v3.int4_ord_ore,
    encrypted_int8 eql_v3.int8_ord_ore,
    encrypted_float8 eql_v3.float8_ord_ore,
    encrypted_date eql_v3.date_ord_ore,
    encrypted_jsonb eql_v3.json,
    encrypted_jsonb_filtered eql_v3.json,
    PRIMARY KEY(id)
);

DROP TABLE IF EXISTS unconfigured_elixir;
CREATE TABLE unconfigured_elixir (
    id serial,
    encrypted_unconfigured eql_v3.text,
    PRIMARY KEY(id)
);
