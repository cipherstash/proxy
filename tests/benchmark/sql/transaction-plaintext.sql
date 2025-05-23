\set a random(1, 100000)
\set b random(1, 100000)
\set c random(1, 100000)

\set username hash_fnv1a(:a)

\set email hash_fnv1a(:b)

\set email_update hash_fnv1a(:c)


BEGIN;

INSERT INTO benchmark_plaintext(username, email) VALUES (:username, :email);

SELECT username FROM benchmark_plaintext WHERE email = :email;

UPDATE benchmark_plaintext SET email = :email_update WHERE username = :username;

SELECT username FROM benchmark_plaintext WHERE email = :email_update;

END;

