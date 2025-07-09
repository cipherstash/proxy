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


def test_jsonb_contained_by():
    val = {"key": "value"}
    column = "encrypted_jsonb"
    select_fragment = "%s <@ encrypted_jsonb"
    tests = [
        (val, True),
        ({"key": "different value"}, False)
    ]

    for (param, expected) in tests:
        params = [json.dumps(param)]
        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(json.dumps(val), column,
                    select_fragment=select_fragment,
                    select_params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_contains():
    val = {"key": "value"}
    column = "encrypted_jsonb"
    select_fragment = "encrypted_jsonb @> %s"
    tests = [
        (val, True),
        ({"key": "different value"}, False)
    ]

    for (param, expected) in tests:
        params = [json.dumps(param)]
        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(json.dumps(val), column,
                    select_fragment=select_fragment,
                    select_params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_extract_simple():
    expected = "value"
    val = {"key": expected}
    column = "encrypted_jsonb"
    select_fragment_template = "{}->'{}'"
    accessors = [
        'key',
        '$.key',  # Undocumented JSONPath selector
    ]

    for accessor in accessors:
        select_fragment = select_fragment_template.format(column, accessor)

        print("Testing field: {}, expecting: {}".format(
            accessor, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(json.dumps(val), column,
                    select_fragment=select_fragment,
                    select_params=[],
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_extract_parameterised():
    val = {
        "string": "hello",
        "number": 42,
        "nested": {
            "number": 1815,
            "string": "world",
        },
        "array_string": ["hello", "world"],
        "array_number": [42, 84],
    }
    column = "encrypted_jsonb"
    select_fragment = "encrypted_jsonb->%s"
    tests = [
        ("string", "hello"),
        ("number", 42),
        ("array_string", ["hello", "world"]),
        ("array_number", [42, 84]),
        ("nested", {"number": 1815, "string": "world"}),
        # TODO: Test ("nonexistentkey", None)
    ]

    for (param, expected) in tests:
        # JSONPath selectors *also* work with EQL extract, but are undocumented
        for accessor in [param, "$." + param]:
            print("Testing accessor: {}, expecting: {}".format(accessor, expected))

            for (binary, prepare) in product([True, None], repeat=2):
                execute(json.dumps(val), column,
                        select_fragment=select_fragment,
                        select_params=[accessor],
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
                expected_result = expected if expected is not None else val

                assert result_id == id
                assert result == expected_result
