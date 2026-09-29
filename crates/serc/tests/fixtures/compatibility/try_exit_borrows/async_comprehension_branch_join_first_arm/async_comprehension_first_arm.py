async def f(flag, stream):
    if flag:
        result = [item async for item in stream]
    else:
        result = []
    return consume(item for item in result)
