async def f(call, first, second, should_raise):
    try:
        return await call()
    except RuntimeError as error:
        if first or (second and should_raise(error)):
            raise
