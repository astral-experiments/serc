async def target(client, record):
    try:
        await client.work()
    finally:
        result = await client.cleanup()
        assert not result.retry
