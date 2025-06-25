
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


SELECT eql_v2.add_encrypted_constraint('encrypted', 'encrypted_text');

SELECT eql_v2.migrate_config();
SELECT eql_v2.activate_config();
