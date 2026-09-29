async def f(stop, left, right):
    try:
        if stop:
            step_0()
            return
    finally:
        if left and right:
            cleanup()
