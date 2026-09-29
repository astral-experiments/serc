async def collect(value, manager, items, flag):
    while flag:
        try:
            try:
                break
            finally:
                async with manager:
                    pass
        finally:
            pass
        after_iteration(value)
    after_loop(value)
