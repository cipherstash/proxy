TRUNCATE TABLE cs_configuration_v1;

DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY,
    email jsonb,
    PRIMARY KEY(id)
);

SELECT cs_add_index_v1(
  'users',
  'email',
  'unique',
  'text'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();