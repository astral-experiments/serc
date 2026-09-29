async def collect(value, manager, items, flag):
    for item in items:
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
