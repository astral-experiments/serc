async def f(items, stop, ready):
    try:
        async for item in items:
            if stop:
                step_0()
                step_1()
                step_2()
                yield ready
                return
    finally:
        if not ready:
            cleanup()
