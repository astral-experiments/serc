async def f():
    try:
        await source
    finally:
        if test:
            await other
