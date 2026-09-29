async def f(value, cleanup):
    try:
        return await value()
    finally:
        raise await cleanup()
