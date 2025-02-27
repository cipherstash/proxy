import json
import os
import psycopg
from psycopg.types import TypeInfo
from psycopg.types.hstore import register_hstore
from psycopg.types.range import Range, RangeInfo, register_range
from psycopg.types.json import Json
from psycopg.types.json import Jsonb
import random

# CREATE USER cipherstash WITH PASSWORD 'password';
# GRANT ALL PRIVILEGES ON DATABASE "cipherstash" to cipherstash;
# GRANT ALL ON SCHEMA public TO cipherstash;
# GRANT ALL ON DATABASE cipherstash TO cipherstash;
# ALTER DATABASE cipherstash OWNER TO cipherstash;


username = os.environ.get("CS_DATABASE__USERNAME")
password = os.environ.get("CS_DATABASE__PASSWORD")
database = os.environ.get("CS_DATABASE__NAME")
host = os.environ.get("CS_DATABASE__HOST")
port = 6432
# port = 5432

connection_str = "postgres://{}:{}@{}:{}/{}".format(username, password, host, port, database)
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

                try:
                    cursor.execute(sql, [id, val])
                    # Unreachable
                    assert(false)

                except Exception as err:
                    msg = str(err)
                    assert(msg.find('column "encrypted_unconfigured" of relation "encrypted" does not exist') == 0)


