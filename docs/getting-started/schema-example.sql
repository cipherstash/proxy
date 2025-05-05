TRUNCATE TABLE public.eql_v1_configuration;

-- Exciting cipherstash table
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    encrypted_email cs_encrypted_v1,
    encrypted_dob cs_encrypted_v1,
    encrypted_salary cs_encrypted_v1
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
