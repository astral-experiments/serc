async def f(flag):
    count = 0
    yield count
    with make():
        async with amake() as conn:
            try:
                work(conn)
            finally:
                return
    pause()
