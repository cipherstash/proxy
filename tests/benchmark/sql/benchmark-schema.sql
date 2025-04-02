TRUNCATE TABLE cs_configuration_v1;

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
    email cs_encrypted_v1
);

SELECT cs_add_column_v1(
  'benchmark_encrypted',
  'email'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();
