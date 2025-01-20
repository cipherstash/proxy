TRUNCATE TABLE cs_configuration_v1;

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
    encrypted_text cs_encrypted_v1,
    encrypted_bool cs_encrypted_v1,
    encrypted_int2 cs_encrypted_v1,
    encrypted_int4 cs_encrypted_v1,
    encrypted_int8 cs_encrypted_v1,
    encrypted_float8 cs_encrypted_v1,
    encrypted_date cs_encrypted_v1,
    PRIMARY KEY(id)
);


SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_text',
  'unique',
  'text'
);


SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_text',
  'match',
  'text'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_bool',
  'unique',
  'boolean'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int2',
  'unique',
  'small_int'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int2',
  'ore',
  'small_int'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int4',
  'unique',
  'int'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int4',
  'ore',
  'int'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int8',
  'unique',
  'big_int'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_int8',
  'ore',
  'big_int'
);


SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_float8',
  'unique',
  'double'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_float8',
  'ore',
  'double'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_date',
  'unique',
  'date'
);

SELECT cs_add_index_v1(
  'encrypted',
  'encrypted_date',
  'ore',
  'date'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();

