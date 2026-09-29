async def f(check, pause, work):
    while True:
        await pause()
        if (
            True
            and check()
        ):
            work()
