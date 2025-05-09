TRUNCATE TABLE public.eql_v1_configuration;

-- Exciting cipherstash table
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    encrypted_email eql_v1_encrypted,
    encrypted_dob eql_v1_encrypted,
    encrypted_salary eql_v1_encrypted
);

SELECT cs_add_index_v1(
  'users',
  'encrypted_email',
  'unique',
  'text'
);

SELECT cs_add_index_v1(
  'users',
  'encrypted_email',
  'match',
  'text'
);

SELECT cs_add_index_v1(
  'users',
  'encrypted_email',
  'ore',
  'text'
);

SELECT cs_add_index_v1(
  'users',
  'encrypted_salary',
  'ore',
  'int'
);

SELECT cs_add_index_v1(
  'users',
  'encrypted_dob',
  'ore',
  'date'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();
