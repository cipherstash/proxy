-- EQL v3 has no `eql_v2_configuration` table (self-configuring domain types),
-- so there is nothing to drop there — just remove the test tables.

DROP TABLE IF EXISTS plaintext;

DROP TABLE IF EXISTS encrypted;

DROP TABLE IF EXISTS unconfigured;

DROP TABLE IF EXISTS encrypted_elixir;

DROP TABLE IF EXISTS unconfigured_elixir;

-- The legacy EQL v2 fixture. The type is dropped after the table that uses it,
-- and is dropped at all because EQL v3 does not own it — the test schema
-- declares it, so the test schema has to take it away again.
DROP TABLE IF EXISTS encrypted_v2_legacy;

DROP TYPE IF EXISTS eql_v2_encrypted;
