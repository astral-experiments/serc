async def f(flag, stream):
    if flag:
        result = []
    else:
        result = [item async for item in stream]
    return consume(item for item in result)
