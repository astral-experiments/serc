async def f(manager, value):
    with manager:
        try:
            value = await read(value)
            if value is not None:
                report(value)
                return value
        except Error:
            recover()
            raise
    after()
