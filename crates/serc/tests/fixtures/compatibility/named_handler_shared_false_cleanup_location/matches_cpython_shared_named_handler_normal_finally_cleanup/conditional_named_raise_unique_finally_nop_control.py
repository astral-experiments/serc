async def f(call, first, cleanup):
    try:
        return await call()
    except RuntimeError as error:
        if first:
            raise
    finally:
        cleanup()
