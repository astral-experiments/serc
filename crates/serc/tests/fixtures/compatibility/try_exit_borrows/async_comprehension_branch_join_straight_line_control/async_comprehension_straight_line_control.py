async def f(stream):
    result = [item async for item in stream]
    return consume(item for item in result)
