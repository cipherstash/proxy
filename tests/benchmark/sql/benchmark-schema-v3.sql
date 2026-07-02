-- EQL v3 port of benchmark-schema.sql.
--
-- GATED: the proxy's eql-mapper cannot speak EQL v3 yet, so this schema is
-- opt-in via `mise run benchmark:setup:v3` (requires EQL v3 installed:
-- `CS_EQL_V3_PATH=... mise run postgres:eql:v3:setup`).
--
-- Differences from the v2 benchmark schema:
--
-- * `email eql_v2_encrypted` becomes `email eql_v3.text_eq`: the encrypted
--   benchmark transaction only exercises equality (`WHERE email = $1`), and
--   `eql_v3.text_eq` is the v3 text domain that requires the `hm`
--   (hash-equality) term.
--
-- * The `eql_v2.add_column` call (and the `eql_v2_configuration` table it
--   populates) has no v3 equivalent: EQL v3 has no database-side
--   configuration. The proxy-side Encrypt config replaces it, and the
--   fail-closed domain CHECK constraints validate stored payloads.
--
-- The pgbench transaction scripts (transaction-*.sql) are version-agnostic
-- plain SQL over column names and are shared with the v2 benchmark.

-- The v2 benchmark schema truncates its configuration table here. v3 has no
-- database-side configuration table, but stale v2 configuration (e.g. the
-- eql_v2.add_column row from a prior v2 benchmark:setup) must not be left
-- pointing at tables whose columns are now v3 domains.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.tables
    WHERE table_schema = 'public' AND table_name = 'eql_v2_configuration'
  ) THEN
    TRUNCATE TABLE public.eql_v2_configuration;
  END IF;
END $$;

DROP TABLE IF EXISTS benchmark_plaintext;
CREATE TABLE benchmark_plaintext (
    id serial primary key,
    username text,
    email text
);

DROP TABLE IF EXISTS benchmark_encrypted;
CREATE TABLE benchmark_encrypted (
    id serial primary key,
    username text,
    email eql_v3.text_eq
);
