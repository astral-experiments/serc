async def f(manager, flag, active, values):
    async with manager:
        try:
            work()
        except Exception as error:
            while active:
                for value in values:
                    consume(value)
        finally:
            cleanup()
