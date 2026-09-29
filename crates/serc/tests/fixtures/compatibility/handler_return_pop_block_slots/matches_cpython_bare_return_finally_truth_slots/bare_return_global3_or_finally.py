async def f(stop, left, right):
    try:
        if stop:
            step_0()
            step_1()
            step_2()
            return
    finally:
        if left or right:
            cleanup()
