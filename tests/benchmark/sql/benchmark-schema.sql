TRUNCATE TABLE public.eql_v2_configuration;

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
    email eql_v2_encrypted,
    encrypted_jsonb_extract eql_v2_encrypted,      -- ~250KB: structured report data
    encrypted_jsonb_full eql_v2_encrypted,         -- ~500KB: raw report data
    encrypted_jsonb_with_ste_vec eql_v2_encrypted  -- STE-VEC benchmarks
);

SELECT eql_v2.add_column(
  'benchmark_encrypted',
  'email'
);

SELECT eql_v2.add_column(
  'benchmark_encrypted',
  'encrypted_jsonb_extract',
  'jsonb'
);

SELECT eql_v2.add_column(
  'benchmark_encrypted',
  'encrypted_jsonb_full',
  'jsonb'
);

SELECT eql_v2.add_search_config(
  'benchmark_encrypted',
  'encrypted_jsonb_with_ste_vec',
  'ste_vec',
  'jsonb',
  '{"prefix": "benchmark_encrypted/encrypted_jsonb_with_ste_vec"}'
);

