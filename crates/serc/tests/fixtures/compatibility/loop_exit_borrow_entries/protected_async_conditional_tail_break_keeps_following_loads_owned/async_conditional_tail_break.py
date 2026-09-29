async def f(items, client, count):
    try:
        async for item in items:
            consume(item)
            count += 1
            if count >= 10:
                break
        if count == 0:
            consume(count)
    finally:
        await client.close()
