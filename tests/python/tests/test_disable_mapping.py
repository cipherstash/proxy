import os
import psycopg
from psycopg.types import TypeInfo
import random
import pytest


conn_params = {
    "user": os.environ.get("CS_DATABASE__USERNAME"),
    "password": os.environ.get("CS_DATABASE__PASSWORD"),
    "dbname": os.environ.get("CS_DATABASE__NAME"),
    "host": os.environ.get("CS_DATABASE__HOST"),
    "port": 6432,
}

connection_str = psycopg.conninfo.make_conninfo(**conn_params)
print("Connection to Tandem with {}".format(connection_str))


def make_id():
    return random.randrange(1, 1000000000)


def test_disable_mapping():

    val = "hello@cipherstash.com";

    execute(val, "encrypted_text")

    execute(val, "encrypted_text", binary=True)

    execute(val, "encrypted_text", binary=True, prepare=True)

    execute(val, "encrypted_text", binary=False, prepare=True)



def execute(val, column, binary=None, prepare=None, expected=None):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            id = make_id()

            print("Testing {} Binary: {} Prepare: {}".format(column, binary, prepare))

            # Insert a value
            sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(column);
            cursor.execute(sql, [id, val], binary=binary, prepare=prepare)


            sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";
            cursor.execute(sql, [])

            # Attempt to nsert a value
            id = make_id()
            sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(column);
            with pytest.raises(psycopg.errors.InvalidTextRepresentation):
                cursor.execute(sql, [id, val], binary=binary, prepare=prepare)


            sql = "SELECT encrypted_text FROM encrypted"
            cursor.execute(sql, [], binary=binary, prepare=prepare)

            row = cursor.fetchone()

            (result,) = row
            assert("encrypted_text" in str(result))


            sql = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false";
            cursor.execute(sql, [])

            sql = "SELECT encrypted_text FROM encrypted"
            cursor.execute(sql, [], binary=binary, prepare=prepare)

            row = cursor.fetchone()

            (result,) = row
            # The prepared statement was prepared while mapping was disabled
            # We only ever decrypt if a statement was mapped and we can link to that statement
            # Data returned from the prepared statement is never decrypted
            if prepare:
                assert("encrypted_text" in str(result))
            else:
                assert(result == val)


            # Bust the prepared statement caching
            sql = "SELECT encrypted_text FROM encrypted WHERE 1=1"
            cursor.execute(sql, [], binary=binary, prepare=prepare)

            row = cursor.fetchone()

            (result,) = row
            assert(result == val)





