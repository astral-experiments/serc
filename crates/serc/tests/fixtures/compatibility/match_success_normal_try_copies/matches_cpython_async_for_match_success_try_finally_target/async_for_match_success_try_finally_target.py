async def f(command, first, last, ready, close, record, kwargs):
    await ready()
    try:
        match command:
            case Kind.FIRST:
                async for item in first(**kwargs):
                    yield item
            case Kind.LAST:
                async for item in last(**kwargs):
                    yield item
            case _:
                raise RuntimeError(command)
    finally:
        await close()
