async def f(items, client, flag):
    try:
        async for item in items:
            if flag:
                break
            else:
                consume(item)
    finally:
        await client.close()
