-- EQL v3 example schema.
--
-- Encrypted columns are self-configuring domain types (`eql_v3_<token>_<cap>`):
-- the column type both marks the column as encrypted and declares which searches
-- it supports. There is no `eql_v2_configuration` table to truncate and no
-- `eql_v2.add_search_config` call — the proxy infers the encrypt config from the
-- column's domain type.

-- Exciting cipherstash table
DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    -- equality + ordering + fuzzy LIKE/ILIKE match
    encrypted_email eql_v3_text_search,
    -- ordering + range comparisons (and equality)
    encrypted_dob eql_v3_date_ord,
    encrypted_salary eql_v3_bigint_ord
);
