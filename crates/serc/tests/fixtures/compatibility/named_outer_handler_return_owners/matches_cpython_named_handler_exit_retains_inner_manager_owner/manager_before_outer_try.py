async def f(manager):
    with manager:
        try:
            try:
                await work()
            except Exception as error:
                report(error)
                record(error)
            return await finish()
        except Exception:
            return None
