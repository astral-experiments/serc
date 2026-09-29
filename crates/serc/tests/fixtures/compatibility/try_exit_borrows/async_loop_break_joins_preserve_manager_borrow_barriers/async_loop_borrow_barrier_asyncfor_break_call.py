async def f(manager, items, flag, x):
    with manager:
        try:
            async for item in items:
                work(item)
                if flag:
                    break
        finally:
            cleanup()
    return x
