async def f(flag):
    count = 0
    async with amake():
        with make() as conn:
            try:
                work(conn)
            except* Error as error:
                work(error)
            finally:
                return 0
    pause()
