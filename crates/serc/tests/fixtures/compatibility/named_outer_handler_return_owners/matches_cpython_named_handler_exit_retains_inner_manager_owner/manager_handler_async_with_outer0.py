async def f(manager):
    async with manager:
        try:
            await work()
        except Exception as error:
            report(error)
            record(error)
        return await finish()
