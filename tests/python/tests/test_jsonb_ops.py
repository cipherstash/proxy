import json
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
        param = json.dumps(param)

        execute(json.dumps(val), column,
                select_fragment=select_fragment,
                select_params=[param],
                expected=expected)

        execute(json.dumps(val).encode(), column,
                select_fragment=select_fragment,
                select_params=[param.encode()],
                expected=expected,
                binary=True)

        execute(json.dumps(val).encode(), column,
                select_fragment=select_fragment,
                select_params=[param.encode()],
                expected=expected,
                binary=True, prepare=True)

        execute(json.dumps(val), column,
                select_fragment=select_fragment,
                select_params=[param],
                expected=expected,
                binary=False, prepare=True)


def test_jsonb_contains():
    val = {"key": "value"}
    column = "encrypted_jsonb"
    select_fragment = "encrypted_jsonb @> %s"
    tests = [
        (val, True),
        ({"key": "different value"}, False)
    ]

    for (param, expected) in tests:
        param = json.dumps(param)

        execute(json.dumps(val), column,
                select_fragment=select_fragment,
                select_params=[param],
                expected=expected)

        execute(json.dumps(val).encode(), column,
                select_fragment=select_fragment,
                select_params=[param.encode()],
                expected=expected,
                binary=True)

        execute(json.dumps(val).encode(), column,
                select_fragment=select_fragment,
                select_params=[param.encode()],
                expected=expected,
                binary=True, prepare=True)

        execute(json.dumps(val), column,
                select_fragment=select_fragment,
                select_params=[param],
                expected=expected,
                binary=False, prepare=True)


def test_jsonb_extract_simple():
    expected = "value"
    val = {"key": expected}
    column = "encrypted_jsonb"
    select_fragment = "encrypted_jsonb->'key'"

    execute(json.dumps(val), column,
            select_fragment=select_fragment,
            select_params=[],
            expected=expected)

    execute(json.dumps(val).encode(), column,
            select_fragment=select_fragment,
            select_params=[],
            expected=expected,
            binary=True)

    execute(json.dumps(val).encode(), column,
            select_fragment=select_fragment,
            select_params=[],
            expected=expected,
            binary=True, prepare=True)

    execute(json.dumps(val), column,
            select_fragment=select_fragment,
            select_params=[],
            expected=expected,
            binary=False, prepare=True)


def test_jsonb_extract():
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
    select_fragment = "encrypted_jsonb->'%s'"
    tests = [
        ("string", "hello"),
        ("number", 42),
        ("array_string", ["hello", "world"]),
        ("array_number", [42, 84]),
        ("nested", {"number": 1815, "string": "world"}),
        ("nonexistent", None),
    ]

    for (param, expected) in tests:

        # JSONPath selectors work with EQL fields
        for accessor in [param, "$." + param]:

            param = json.dumps(param)

            execute(json.dumps(val), column,
                    select_fragment=select_fragment,
                    select_params=[param],
                    expected=expected)

            execute(json.dumps(val).encode(), column,
                    select_fragment=select_fragment,
                    select_params=[param.encode()],
                    expected=expected,
                    binary=True)

            execute(json.dumps(val).encode(), column,
                    select_fragment=select_fragment,
                    select_params=[param.encode()],
                    expected=expected,
                    binary=True, prepare=True)

            execute(json.dumps(val), column,
                    select_fragment=select_fragment,
                    select_params=[param],
                    expected=expected,
                    binary=False, prepare=True)


def make_id():
    return random.randrange(1, 1000000000)


def execute(val, column, binary=None, prepare=None, expected=None,
            select_fragment=None, select_params=[]):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                id = make_id()

                print("Testing {} Binary: {} Prepare: {}".format(
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
