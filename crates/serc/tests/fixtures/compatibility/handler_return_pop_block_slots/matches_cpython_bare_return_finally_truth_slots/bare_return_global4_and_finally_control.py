async def f(stop, left, right):
    try:
        if stop:
            step_0()
            step_1()
            step_2()
            step_3()
            return
    finally:
        if left and right:
            cleanup()
