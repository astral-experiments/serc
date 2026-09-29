async def f(value, other, manager, flag, items, consume, poll):
    async with manager:
        try:
            while poll() is None:
                consume(value)
                consume(other)
        except Error:
            try:
                consume(other)
            except InnerError:
                raise
    return value, other
