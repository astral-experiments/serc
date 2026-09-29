async def f(values):
    try:
        try:
            first()
        except FirstError:
            async for value in values:
                yield value
                if value:
                    return
    except OuterError:
        raise
