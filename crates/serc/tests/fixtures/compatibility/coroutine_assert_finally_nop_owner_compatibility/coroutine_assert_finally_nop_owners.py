async def await_method_cleanup(client, expected):
    try:
        value = await client.read()
        assert value.status == expected
    finally:
        await client.close()


async def call_method_cleanup(client, expected):
    try:
        value = await client.read()
        assert value.status is expected
    finally:
        client.close()


async def assignment_cleanup(client, expected):
    try:
        value = await client.read()
        assert value.status == expected
    finally:
        client.closed = True


async def truthy_assert_cleanup(client):
    try:
        value = await client.read()
        assert value.ready
    finally:
        await client.close()
