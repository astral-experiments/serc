async def f(source, enabled, first, second, finish):
    if enabled:
        async for item in source:
            finish(item)
        if first:
            if second:
                raise ValueError
        elif enabled and second:
            raise ValueError
    finish(source)
    return source
