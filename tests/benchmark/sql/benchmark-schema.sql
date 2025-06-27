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
    email eql_v2_encrypted
);

SELECT eql_v2.add_column(
  'benchmark_encrypted',
  'email'
);

-- SELECT eql_v2.encrypt();
-- SELECT eql_v2.activate();

SELECT eql_v2.migrate_config();
SELECT eql_v2.activate_config();
