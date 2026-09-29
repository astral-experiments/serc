async def f(items, stream):
    result = [item for group in items async for item in stream]
    return consume(item for item in result)
