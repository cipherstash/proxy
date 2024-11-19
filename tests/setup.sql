TRUNCATE TABLE cs_configuration_v1;

DROP TABLE IF EXISTS blah;
CREATE TABLE blah (
    id bigint GENERATED ALWAYS AS IDENTITY,
    vtha jsonb,
    PRIMARY KEY(id)
);

SELECT cs_add_index_v1(
  'blah',
  'vtha',
  'unique',
  'text'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();