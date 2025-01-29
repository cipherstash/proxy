import asyncio
import os
import psycopg
from psycopg.types import TypeInfo
import random
import pytest

username = os.environ.get("CS_DATABASE__USERNAME")
password = os.environ.get("CS_DATABASE__PASSWORD")
database = os.environ.get("CS_DATABASE__NAME")
host = os.environ.get("CS_DATABASE__HOST")
port = 6432

connection_str = "postgres://{}:{}@{}:{}/{}".format(username, password, host, port, database)
print("Connection to Tandem with {}".format(connection_str))


def make_id():
    return random.randrange(1, 1000000000)

@pytest.mark.asyncio
async def test_map_text_async():

    val = "hello@cipherstash.com";

    await execute(val, "encrypted_text")

    await execute(val, "encrypted_text", binary=True)

    await execute(val, "encrypted_text", binary=True, prepare=True)

    await execute(val, "encrypted_text", binary=False, prepare=True)


async def execute(val, column, binary=None, prepare=None):

    count = 0


    async with await psycopg.AsyncConnection.connect(connection_str, autocommit=True) as conn:
        async with conn:
            async with conn.cursor() as cursor:
                async with conn.transaction():

                    for _ in range(25):
                        id = make_id()

                        print("Testing {} Binary: {} Prepare: {}".format(column, binary, prepare))

                        sql = "INSERT INTO encrypted (id, {}) VALUES (%s, %s)".format(column);

                        await cursor.execute(sql, [id, val], binary=binary, prepare=prepare)

                        sql = "SELECT id, {} FROM encrypted WHERE id = %s".format(column);

                        await cursor.execute(sql, [id], binary=binary, prepare=prepare)

                        row = await cursor.fetchone()
                        (result_id, result) = row

                        assert(result_id == id)
                        assert(result == val)

                        count += 1

                    assert(count == 25)

