async def f(flag):
    count = 0
    async with amake():
        with make() as conn:
            try:
                work(conn)
            finally:
                return await awork(count)
    pause()
