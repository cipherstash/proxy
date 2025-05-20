TRUNCATE TABLE public.eql_v1_configuration;

-- Regular old table
DROP TABLE IF EXISTS plaintext;
CREATE TABLE plaintext (
    id bigint,
    plaintext text,
    PRIMARY KEY(id)
);

-- Exciting cipherstash table
DROP TABLE IF EXISTS encrypted;
CREATE TABLE encrypted (
    id bigint,
    plaintext text,
    encrypted_text eql_v1_encrypted,
    encrypted_bool eql_v1_encrypted,
    encrypted_int2 eql_v1_encrypted,
    encrypted_int4 eql_v1_encrypted,
    encrypted_int8 eql_v1_encrypted,
    encrypted_int8_as_biguint eql_v1_encrypted,
    encrypted_float8 eql_v1_encrypted,
    encrypted_date eql_v1_encrypted,
    encrypted_jsonb eql_v1_encrypted,
    PRIMARY KEY(id)
);

DROP TABLE IF EXISTS unconfigured;
CREATE TABLE unconfigured (
    id bigint,
    encrypted_unconfigured eql_v1_encrypted,
    PRIMARY KEY(id)
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_text',
  'unique',
  'text'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_text',
  'match',
  'text'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_text',
  'ore',
  'text'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_bool',
  'unique',
  'boolean'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_bool',
  'ore',
  'boolean'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int2',
  'unique',
  'small_int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int2',
  'ore',
  'small_int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int4',
  'unique',
  'int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int4',
  'ore',
  'int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int8',
  'unique',
  'big_int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int8',
  'ore',
  'big_int'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_int8_as_biguint',
  'unique',
  'big_uint'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_float8',
  'unique',
  'double'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_float8',
  'ore',
  'double'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_date',
  'unique',
  'date'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_date',
  'ore',
  'date'
);

SELECT eql_v1.add_index(
  'encrypted',
  'encrypted_jsonb',
  'ste_vec',
  'jsonb',
  '{"prefix": "encrypted/encrypted_jsonb"}'
);

SELECT eql_v1.encrypt();
SELECT eql_v1.activate();
