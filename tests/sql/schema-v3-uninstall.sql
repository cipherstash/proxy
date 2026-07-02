-- Uninstall for tests/sql/schema-v3.sql (EQL v3 fixture).
--
-- Mirrors schema-uninstall.sql: drops the shared fixture tables. The v3
-- fixture has no configuration table of its own (v3 configuration is
-- client-side), so unlike the v2 uninstall there is no
-- `public.eql_v2_configuration` to drop here.

-- Regular old table
DROP TABLE IF EXISTS plaintext;

-- Exciting cipherstash table
DROP TABLE IF EXISTS encrypted;

DROP TABLE IF EXISTS unconfigured;
