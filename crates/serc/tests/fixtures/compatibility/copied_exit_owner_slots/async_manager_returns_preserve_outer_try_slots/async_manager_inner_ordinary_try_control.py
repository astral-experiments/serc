async def run(manager, flag):
    try:
        async with manager:
            try:
                return await work()
            except ValueError:
                report()
                raise
    finally:
        if flag is not None:
            await cleanup(flag)
