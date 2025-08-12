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


def test_jsonb_gt_simple():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment_template = "{}->'{}' > %s"
    tests = [
        ("string", "C", vals[3:]),
        ("number", 4, vals[4:]),
    ]

    for (field, param, expected) in tests:
        where_fragment = where_fragment_template.format(column, field)
        params = [json.dumps(param)]

        print("Testing field: {}, param: {}, expecting: {}".format(
            field, param, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_gt_parameterised():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment = "{}->%s > %s".format(column)
    tests = [
        (["string", json.dumps("C")], vals[3:]),
        (["number", json.dumps(4)], vals[4:]),
    ]

    for (params, expected) in tests:

        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_gte_simple():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment_template = "{}->'{}' >= %s"
    tests = [
        ("string", "C", vals[2:]),
        ("number", 4, vals[3:]),
    ]

    for (field, param, expected) in tests:
        where_fragment = where_fragment_template.format(column, field)
        params = [json.dumps(param)]

        print("Testing field: {}, params: {}, expecting: {}".format(
            field, params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_gte_parameterised():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment = "{}->%s >= %s".format(column)
    tests = [
        (["string", json.dumps("C")], vals[2:]),
        (["number", json.dumps(4)], vals[3:]),
    ]

    for (params, expected) in tests:

        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_lt_simple():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment_template = "{}->'{}' < %s"
    tests = [
        ("string", "C", vals[0:2]),
        ("number", 4, vals[0:3]),
    ]

    for (field, param, expected) in tests:
        where_fragment = where_fragment_template.format(column, field)
        params = [json.dumps(param)]

        print("Testing field: {}, params: {}, expecting: {}".format(
            field, params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_lt_parameterised():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment = "{}->%s < %s".format(column)
    tests = [
        (["string", json.dumps("C")], vals[0:2]),
        (["number", json.dumps(4)], vals[0:3]),
    ]

    for (params, expected) in tests:

        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_lte_simple():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment_template = "{}->'{}' <= %s"
    tests = [
        ("string", "C", vals[0:3]),
        ("number", 4, vals[0:4]),
    ]

    for (field, param, expected) in tests:
        where_fragment = where_fragment_template.format(column, field)
        params = [json.dumps(param)]

        print("Testing field: {}, params: {}, expecting: {}".format(
            field, params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def test_jsonb_lte_parameterised():
    vals = make_values()
    column = "encrypted_jsonb"
    where_fragment = "{}->%s <= %s".format(column)
    tests = [
        (["string", json.dumps("C")], vals[0:3]),
        (["number", json.dumps(4)], vals[0:4]),
    ]

    for (params, expected) in tests:

        print("Testing params: {}, expecting: {}".format(
            params, expected))

        for (binary, prepare) in product([True, None], repeat=2):
            execute(list(map(json.dumps, vals)), column,
                    where_fragment=where_fragment,
                    params=params,
                    expected=expected,
                    binary=binary,
                    prepare=prepare)


def make_id():
    return random.randrange(1, 1000000000)


def make_values():
    values = []
    for n in range(1, 6):  # 1..=5
        values.append({
            "string": chr(ord("A") + (n - 1)),
            "number": n,
        })
    return values


def execute(vals, column, binary=None, prepare=None, expected=None,
            where_fragment=None, params=[]):
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():
                print("... for column {}, with binary: {}, prepare: {}".format(
                    column, binary, prepare))

                ids = []
                for val in vals:
                    id = make_id()
                    ids.append(id)

                    sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(
                        column)
                    cursor.execute(sql, [id, val], binary=binary, prepare=prepare)

                sql = "SELECT encrypted_jsonb FROM encrypted WHERE id = ANY(%s) AND {}".format(
                    where_fragment)
                cursor.execute(
                    sql, [ids] + params,
                    binary=binary, prepare=prepare)

                rows = list(map(
                    lambda row_tuple: row_tuple[0], cursor.fetchall()))

                rows.sort(key=lambda r: r["number"])
                expected.sort(key=lambda r: r["number"])

                assert rows == expected
