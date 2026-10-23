# psycopg sends a COPY with no parameters as a simple Query.
# Proxy forwards a COPY into a table with no encrypted columns, and refuses one into an encrypted column, as it does not encrypt copy data.
import os
import psycopg
import pytest
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
    return random.randrange(1, 1000000000)


def test_copy_into_a_plaintext_table():
    first, second = make_id(), make_id()
    with psycopg.connect(connection_str, autocommit=True) as conn:
        with conn.cursor() as cursor:
            with cursor.copy("COPY plaintext (id, plaintext) FROM STDIN") as copy:
                copy.write_row((first, "alice"))
                copy.write_row((second, "bob"))

            cursor.execute(
                "SELECT plaintext FROM plaintext WHERE id IN (%s, %s) ORDER BY plaintext",
                [first, second],
            )
            assert [row[0] for row in cursor.fetchall()] == ["alice", "bob"]


def test_copy_into_an_encrypted_column_is_refused():
    with psycopg.connect(connection_str, autocommit=True) as conn:
        with conn.cursor() as cursor:
            with pytest.raises(psycopg.Error, match=r"COPY into encrypted column 'encrypted_text' of table 'encrypted' is not supported"):
                with cursor.copy("COPY encrypted (id, encrypted_text) FROM STDIN") as copy:
                    copy.write_row((make_id(), "alice@example.com"))

            # The connection is still usable
            cursor.execute("SELECT 1")
            assert cursor.fetchone() == (1,)
