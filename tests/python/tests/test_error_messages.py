import json
import os
import psycopg
from psycopg.types import TypeInfo
from psycopg.types.hstore import register_hstore
from psycopg.types.range import Range, RangeInfo, register_range
from psycopg.types.json import Json
from psycopg.types.json import Jsonb
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
print("Connection to Tandem with {}".format(connection_str))

def make_id():
    return random.randrange(1, 1000000000)


def test_encrypted_column_not_defined_in_schema():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                id = make_id()
                val = "hello@cipherstash.com";

                sql = "INSERT INTO encrypted (id, encrypted_unconfigured) VALUES (%s, %s)"


                with pytest.raises(psycopg.Error, match=r'relation ".*" does not exist'):
                    cursor.execute(sql, [id, val])


def test_encrypted_column_with_no_configuration():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                id = make_id()

                val = '{"hello": "world"}'

                sql = "INSERT INTO unconfigured (id, encrypted_unconfigured) VALUES (%s, %s)"

                with pytest.raises(psycopg.Error, match=r"Column 'encrypted_unconfigured' in table 'unconfigured' has no Encrypt configuration."):
                    cursor.execute(sql, [id, val])


def test_mapper_unsupported_parameter_type():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                id = make_id()
                val = 2025

                sql = "INSERT INTO encrypted (id, encrypted_date) VALUES (%s, %s)"

                with pytest.raises(psycopg.Error, match='#mapping-invalid-parameter'):
                    cursor.execute(sql, [id, val])


def test_invalid_sql_statement():
    with psycopg.connect(connection_str, autocommit=True) as conn:

        with conn.cursor() as cursor:

            with conn.transaction():

                id = make_id()
                val = 2025

                sql = "INSERT INTO encrypted id, encrypted_date VALUES (%s, %s)"

                with pytest.raises(psycopg.Error, match='#mapping-invalid-sql-statement'):
                    cursor.execute(sql, [id, val])
