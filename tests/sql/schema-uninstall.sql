-- EQL v3 has no `eql_v2_configuration` table (self-configuring domain types),
-- so there is nothing to drop there — just remove the test tables.

DROP TABLE IF EXISTS plaintext;

DROP TABLE IF EXISTS encrypted;

DROP TABLE IF EXISTS unconfigured;

DROP TABLE IF EXISTS encrypted_elixir;

DROP TABLE IF EXISTS unconfigured_elixir;
