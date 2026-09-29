async def f(pending, source, limit):
    try:
        while pending or source.ready:
            if pending:
                pending = await source.take()
                continue
            if source.ready:
                await source.fill()
                continue
            break
        source.finished()
    except (ValueError, TypeError):
        await source.cancel()
        raise
    values = list(value for value in source.values)
    if limit:
        return source.finish(values, limit)
    return values
