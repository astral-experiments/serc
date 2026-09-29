async def f(items, client):
    try:
        async for item in items:
            if item.ready:
                consume(item)
            if item.done:
                consume(item)
                break
    finally:
        await client.close()
