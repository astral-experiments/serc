async def f(value, make, touch, cleanup):
    try:
        result = make(value)
        try:
            await touch(result)
        except BaseException:
            await cleanup(result)
            raise
        return result
    finally:
        cleanup(value)
