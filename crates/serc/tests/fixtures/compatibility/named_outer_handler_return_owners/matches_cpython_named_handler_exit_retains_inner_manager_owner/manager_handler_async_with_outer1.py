async def f(manager):
    try:
        async with manager:
            try:
                await work()
            except Exception as error:
                report(error)
                record(error)
            return await finish()
    except Exception:
        return None
