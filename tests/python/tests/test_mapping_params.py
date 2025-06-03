import json
import os
import psycopg
from psycopg.types import TypeInfo
from psycopg.types.hstore import register_hstore
from psycopg.types.range import Range, RangeInfo, register_range
from psycopg.types.json import Json
from psycopg.types.json import Jsonb
import random

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

def test_map_text():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                val = "hello@cipherstash.com";

                execute(val, "encrypted_text")

                execute(val, "encrypted_text", binary=True)

                execute(val, "encrypted_text", binary=True, prepare=True)

                execute(val, "encrypted_text", binary=False, prepare=True)


def test_map_int2():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                val = 42;

                execute(val, "encrypted_int2")

                execute(val, "encrypted_int2", binary=True)

                execute(val, "encrypted_int2", binary=True, prepare=True)

                execute(val, "encrypted_int2", binary=False, prepare=True)


def test_map_int4():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                val = 42;

                execute(val, "encrypted_int4")

                execute(val, "encrypted_int4", binary=True)

                execute(val, "encrypted_int4", binary=True, prepare=True)

                execute(val, "encrypted_int4", binary=False, prepare=True)


def test_map_int4_with_large_int():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                val = 42000000;

                execute(val, "encrypted_int4")

                execute(val, "encrypted_int4", binary=True)

                execute(val, "encrypted_int4", binary=True, prepare=True)

                execute(val, "encrypted_int4", binary=False, prepare=True)


def test_map_int8():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                val = 42;

                execute(val, "encrypted_int8")

                execute(val, "encrypted_int8", binary=True)

                execute(val, "encrypted_int8", binary=True, prepare=True)

                execute(val, "encrypted_int8", binary=False, prepare=True)

def test_map_jsonb():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                val = {"key": "value"}

                execute(json.dumps(val), "encrypted_jsonb", expected=val)

                execute(json.dumps(val).encode(), "encrypted_jsonb", binary=True, expected=val)

                execute(json.dumps(val).encode(), "encrypted_jsonb", binary=True, prepare=True, expected=val)

                execute(json.dumps(val), "encrypted_jsonb", binary=False, prepare=True, expected=val)



def execute(val, column, binary=None, prepare=None, expected=None):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                id = make_id()

                # id = 167524859
                print("Testing {} Binary: {} Prepare: {}".format(column, binary, prepare))

                sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(column);

                cursor.execute(sql, [id, val], binary=binary, prepare=prepare)

                sql = "SELECT id, {} FROM encrypted WHERE id = %s".format(column);
                cursor.execute(sql, [id], binary=binary, prepare=prepare)

                row = cursor.fetchone()

                (result_id, result) = row
                expected_result = expected if expected is not None else val

                assert(result_id == id)
                assert(result == expected_result)
