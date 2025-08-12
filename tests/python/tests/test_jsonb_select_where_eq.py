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


def test_jsonb_eq_simple():
    val = {"key": "value"}
    column = "encrypted_jsonb"
    where_fragments = [
        "{}->'key' = %s".format(column),
        "jsonb_path_query_first({}, 'key') = %s".format(column),
    ]
    tests = [
        ("value", val),
        ("different value", None),
    ]

    for where_fragment in where_fragments:
        for (param, expected) in tests:
            params = [json.dumps(param)]

            print("Testing fragment: {}, params: {}, expecting: {}".format(
                where_fragment, params, expected))

            for (binary, prepare) in product([True, None], repeat=2):
                execute(json.dumps(val), column,
                        where_fragment=where_fragment,
                        params=params,
                        expected=expected,
                        binary=binary,
                        prepare=prepare)


def test_jsonb_parameterised():
    val = {
        "key": "value",
        "array_number": [1, 2, 3],
        "array_string": ["hello", "world"],
        "nested": {"foo": "bar"},
    }
    column = "encrypted_jsonb"
    where_fragments = [
        "{} -> %s = %s".format(column),
        "jsonb_path_query_first({}, %s) = %s".format(column),
    ]
    tests = [
        (["key", json.dumps("value")], val),
        (["$.key", json.dumps("value")], val),
        (["key", json.dumps("different value")], None),
        (["$.nested.foo", json.dumps("bar")], val),
        (["$.array_number[0]", json.dumps(1)], val),
        # TODO: Test ["$.array_number", json.dumps([1, 2, 3])]
        # TODO: Test an incorrect/missing key
    ]

    for where_fragment in where_fragments:
        for (params, expected) in tests:
            print("Testing fragment: {}, params: {}, expecting: {}".format(
                where_fragment, params, expected))

            for (binary, prepare) in product([True, None], repeat=2):
                execute(json.dumps(val), column,
                        where_fragment=where_fragment,
                        params=params,
                        expected=expected,
                        binary=binary,
                        prepare=prepare)


def test_jsonb_eq_numeric():
    val = {"string": "C", "number": 3}
    column = "encrypted_jsonb"
    where_fragment = "{}->'number' = %s".format(column)
    tests = [
        (3, val),
        (5, None),
    ]

    for (param, expected) in tests:
        params = [json.dumps(param)]

        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def make_id():
    return random.randrange(1, 1000000000)


def execute(val, column, binary=None, prepare=None, expected=None,
            where_fragment=None, params=[]):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                id = make_id()

                print("... for column {}, with binary: {}, prepare: {}".format(
                    column, binary, prepare))

                sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(
                    column)

                cursor.execute(sql, [id, val], binary=binary, prepare=prepare)

                sql = "SELECT id, encrypted_jsonb FROM encrypted WHERE id = %s AND {}".format(
                    where_fragment)
                cursor.execute(
                    sql, [id] + params,
                    binary=binary, prepare=prepare)

                row = cursor.fetchone()

                # If expected is None, we mean that there is no result.
                # If expected is not None, we mean that we expect a row.
                if expected is None:
                    assert row == expected
                else:
                    (result_id, result) = row
                    assert result_id == id
                    assert result == expected
