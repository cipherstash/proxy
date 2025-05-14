TRUNCATE TABLE public.eql_v1_configuration;

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
    email eql_v1_encrypted
);

SELECT eql_v1.add_column(
  'benchmark_encrypted',
  'email'
);

SELECT eql_v1.encrypt();
SELECT eql_v1.activate();

