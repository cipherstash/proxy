"""Reuse of the unnamed prepared statement across mapped and unmapped statements.

A Parse rebinds its name, so whatever the name referred to before is gone. If
Proxy keeps the previous statement cached when it does not map the new one —
`BEGIN`/`COMMIT` need no type check — the next Bind for that name is rewritten
against a statement the client never parsed, and the param counts do not line up:

    FATAL: Rewritten statement binds parameter 1, but only 0 were provided

psycopg with `prepare=False` uses the unnamed statement for every execute, which
is the same shape pgbench drives in extended mode.
"""

import os
import psycopg
import random

conn_params = {
    "user": os.environ.get("CS_DATABASE__USERNAME"),
    "password": os.environ.get("CS_DATABASE__PASSWORD"),
    "dbname": os.environ.get("CS_DATABASE__NAME"),
    "host": os.environ.get("CS_DATABASE__HOST"),
    "port": 6432,
}

connection_str = psycopg.conninfo.make_conninfo(**conn_params)


def make_id():
    return random.randint(1, 1_000_000_000)


def test_unmapped_statement_after_mapped_one_on_the_unnamed_statement():
    with psycopg.connect(connection_str, autocommit=True) as conn:
        with conn.cursor() as cursor:
            # A mapped statement with one param, on the unnamed statement.
            cursor.execute(
                "SELECT id FROM encrypted WHERE encrypted_text = %s",
                ["hello@cipherstash.com"],
                prepare=False,
            )
            cursor.fetchall()

            # An unmapped statement with no params, rebinding the same name.
            # Before the fix this was rewritten against the SELECT above.
            cursor.execute("BEGIN", prepare=False)
            cursor.execute("COMMIT", prepare=False)

            # And the connection is still usable.
            cursor.execute(
                "SELECT id FROM encrypted WHERE encrypted_text = %s",
                ["hello@cipherstash.com"],
                prepare=False,
            )
            cursor.fetchall()


def test_transaction_of_mapped_statements_then_commit():
    """The pgbench shape: a transaction whose body is mapped and whose END is not."""
    with psycopg.connect(connection_str, autocommit=True) as conn:
        with conn.cursor() as cursor:
            id = make_id()
            val = "unnamed-{}@cipherstash.com".format(id)

            cursor.execute("BEGIN", prepare=False)

            cursor.execute(
                "INSERT INTO encrypted (id, encrypted_text) VALUES (%s, %s)",
                [id, val],
                prepare=False,
            )
            cursor.execute(
                "SELECT encrypted_text FROM encrypted WHERE encrypted_text = %s",
                [val],
                prepare=False,
            )
            assert cursor.fetchone() == (val,)

            cursor.execute("END", prepare=False)

            cursor.execute("DELETE FROM encrypted WHERE id = %s", [id], prepare=False)
