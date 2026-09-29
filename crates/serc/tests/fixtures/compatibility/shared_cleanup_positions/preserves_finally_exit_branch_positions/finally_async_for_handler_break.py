async def f(work, values, get):
    try:
        await work()
    finally:
        for value in values:
            try:
                await get(value)
            except LookupError:
                break
