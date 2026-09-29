async def f(flag, stream):
    if flag:
        result = [item async for item in stream]
    else:
        result = [other async for other in stream]
    return consume(item for item in result)
