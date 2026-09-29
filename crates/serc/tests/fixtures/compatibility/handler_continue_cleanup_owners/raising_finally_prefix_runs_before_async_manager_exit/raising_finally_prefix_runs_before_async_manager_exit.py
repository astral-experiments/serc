async def f(manager, flag, skip, observe, report):
    while flag():
        async with manager():
            try:
                if skip():
                    continue
                await observe()
            finally:
                report("cleanup")
                raise RuntimeError("stop")
