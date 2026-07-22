-- EQL v3: the encrypted column is a self-configuring domain type; there is no
-- `eql_v2_configuration` table or `eql_v2.add_column` call.

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
    email eql_v3_text_search
);
