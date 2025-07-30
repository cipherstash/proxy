import json
import os
import psycopg
import random
# import pytest

conn_params = {
    "user": os.environ.get("CS_DATABASE__USERNAME"),
    "password": os.environ.get("CS_DATABASE__PASSWORD"),
    "dbname": os.environ.get("CS_DATABASE__NAME"),
    "host": os.environ.get("CS_DATABASE__HOST"),
    "port": 6432,
}

connection_str = psycopg.conninfo.make_conninfo(**conn_params)

print("Connection to Proxy with {}".format(connection_str))


#@pytest.mark.skip()
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
            param = json.dumps(param)

            print("Testing fragment: {}, param: {}, expecting: {}".format(
                where_fragment, param, expected))

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=[param],
                    expected=expected)

            execute(json.dumps(val).encode(), column,
                    where_fragment=where_fragment,
                    params=[param.encode()],
                    expected=expected,
                    binary=True)

            execute(json.dumps(val).encode(), column,
                    where_fragment=where_fragment,
                    params=[param.encode()],
                    expected=expected,
                    binary=True, prepare=True)

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=[param],
                    expected=expected,
                    binary=False, prepare=True)


#@pytest.mark.skip(reason="All parameterised jsonb_col->%s or jsonb_path_query*() calls fail or hang")
def test_jsonb_parameterised():
    val = {"key": "value"}
    column = "encrypted_jsonb"
    where_fragments = [
        "{} -> %s = %s".format(column),
        "jsonb_path_query_first({}, %s) = %s".format(column),
    ]
    tests = [
        (["key", json.dumps("value")], val),
        (["key", json.dumps("different value")], None),
        # TODO: diff_key?
    ]

    for where_fragment in where_fragments:
        for (params, expected) in tests:
            #params = list(map(json.dumps, params))

            print("Testing fragment: {}, params: {}, expecting: {}".format(
                where_fragment, params, expected))

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected)

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=True)

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=True, prepare=True)

            execute(json.dumps(val), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=False, prepare=True)


#@pytest.mark.skip()
def test_jsonb_eq_numeric():
    val = {"string": "C", "number": 3}
    column = "encrypted_jsonb"
    where_fragment = "{}->'number' = %s".format(column)
    tests = [
        (3, val),
        (5, None),
    ]

    for (param, expected) in tests:
        param = json.dumps(param)

        print("Testing param: {}, expecting: {}".format(param, expected))

        execute(json.dumps(val), column,
                where_fragment=where_fragment,
                params=[param],
                expected=expected)

        execute(json.dumps(val), column,
                where_fragment=where_fragment,
                params=[param],
                expected=expected,
                binary=True)

        execute(json.dumps(val), column,
                where_fragment=where_fragment,
                params=[param],
                expected=expected,
                binary=True, prepare=True)

        execute(json.dumps(val), column,
                where_fragment=where_fragment,
                params=[param],
                expected=expected,
                binary=False, prepare=True)


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
