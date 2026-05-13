TRUNCATE TABLE public.eql_v2_configuration;

-- Exciting cipherstash table
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    encrypted_email eql_v2_encrypted,
    encrypted_dob eql_v2_encrypted,
    encrypted_salary eql_v2_encrypted
);

SELECT eql_v2.add_search_config(
  'users',
  'encrypted_email',
  'unique',
  'text'
);

SELECT eql_v2.add_search_config(
  'users',
  'encrypted_email',
  'match',
  'text'
);

-- 'ore' supports ordering and range comparisons. 'ope' is a drop-in
-- alternative with the same operator support — choose one per column.
SELECT eql_v2.add_search_config(
  'users',
  'encrypted_email',
  'ore',
  'text'
);

SELECT eql_v2.add_search_config(
  'users',
  'encrypted_salary',
  'ore',
  'int'
);

SELECT eql_v2.add_search_config(
  'users',
  'encrypted_dob',
  'ore',
  'date'
);
