async def f(value):
    try:
        await work()
    finally:
        if value is not None:
            await cleanup(value)
