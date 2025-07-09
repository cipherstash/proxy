import json
import os
import psycopg
import random
from itertools import product

conn_params = {
    "user": os.environ.get("CS_DATABASE__USERNAME"),
    "password": os.environ.get("CS_DATABASE__PASSWORD"),
    "dbname": os.environ.get("CS_DATABASE__NAME"),
    "host": os.environ.get("CS_DATABASE__HOST"),
    "port": 6432,
}

connection_str = psycopg.conninfo.make_conninfo(**conn_params)

print("Connection to Proxy with {}".format(connection_str))

# Common test data
val = {
    "key": "value",
    "number": 42,
    "array_number": [3, 2, 1],
    "array_string": ["hello", "world"],
    "nested": {"foo": "bar", "number": 1312},
}


def test_array_wildcard_string():
    select_jsonb("encrypted_jsonb", val, "$.array_string[*]", "hello")


def test_array_wildcard_number():
    select_jsonb("encrypted_jsonb", val, "$.array_number[*]", 3)


def test_string():
    select_jsonb("encrypted_jsonb", val, "$.nested.foo", "bar")


def test_value():
    select_jsonb("encrypted_jsonb", val, "$.nested", val['nested'])


def test_with_unknown():
    select_jsonb("encrypted_jsonb", val, "$.nonexistent", None)


def test_value_with_alias():
    select_jsonb("encrypted_jsonb", val, "$.nested", val['nested'],
                 alias="selected")


def select_jsonb(column, value, selector, expected, alias=None):
    alias = "AS {}".format(alias) if alias is not None else ""
    tests = [
        ("jsonb_path_query_first({}, '{}') {}".format(column, selector, alias), []),
        ("jsonb_path_query_first({}, %s) {}".format(column, alias), [selector]),
    ]

    for (select_fragment, params) in tests:
        print("Testing fragment: {}, params: {}, expecting: {}".format(
            select_fragment, params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(json.dumps(value), column,
                    select_fragment=select_fragment,
                    select_params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def make_id():
    return random.randrange(1, 1000000000)


def execute(val, column, binary=None, prepare=None, expected=None,
            select_fragment=None, select_params=[]):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                id = make_id()

                print("... for column {}, with binary: {}, prepare: {}".format(
                    column, binary, prepare))

                sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(
                    column)

                cursor.execute(sql, [id, val], binary=binary, prepare=prepare)

                sql = "SELECT id, {} FROM encrypted WHERE id = %s".format(
                    select_fragment)
                cursor.execute(
                    sql, (select_params + [id]),
                    binary=binary, prepare=prepare)

                row = cursor.fetchone()

                (result_id, result) = row
                assert result_id == id
                assert result == expected
