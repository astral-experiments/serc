async def f(outer, inner, value, work):
    try:
        if outer:
            try:
                await work(value)
                if inner:
                    await work(inner)
            except ValueError:
                await work(None)
                raise
    except Exception:
        await work(None)
    await work(value)
    return value
