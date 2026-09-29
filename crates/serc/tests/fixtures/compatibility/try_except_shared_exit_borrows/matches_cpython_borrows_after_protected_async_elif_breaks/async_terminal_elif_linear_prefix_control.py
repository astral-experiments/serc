async def collect(stream, manager, close, report):
    result = None
    values = []
    try:
        async with manager:
            async for message in stream:
                if message.kind == 0:
                    result = message.value
                elif message.kind == 1:
                    values.append(message.value)
                elif message.kind == 2:
                    raise ValueError(message.name)
                elif message.kind == 3:
                    result = message.value
                    report(result)
                    break
    finally:
        await close(stream)
    if result is None:
        raise ValueError("no result")
    report(stream, result, values)
    return result, values
