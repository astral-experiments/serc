import asyncio

async def run(client, expected, left, right):
    try:
        await client.work()
    except Exception as caught:
        assert caught is expected
    else:
        assert expected is None
        client.check(expected)
    await asyncio.gather(*list(client.pending))
    return left, right
