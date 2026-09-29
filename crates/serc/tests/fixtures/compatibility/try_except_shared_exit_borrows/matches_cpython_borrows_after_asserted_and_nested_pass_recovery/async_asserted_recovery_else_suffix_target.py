import asyncio

async def run(client, expected, left, right):
    try:
        await client.work()
    except Exception as caught:
        assert caught is expected
    else:
        assert expected is None
    await asyncio.gather(*list(client.pending))
    return left, right
