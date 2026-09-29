async def run(manager, flag):
    try:
        try:
            async with manager:
                return await work()
        except ValueError:
            report()
            raise
    except RuntimeError:
        report_outer()
        raise
    finally:
        if flag is not None:
            await cleanup(flag)
