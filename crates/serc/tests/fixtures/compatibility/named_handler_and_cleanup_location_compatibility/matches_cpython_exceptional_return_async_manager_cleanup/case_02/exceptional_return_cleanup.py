async def f(flag):
    count = 0
    async with amake(), amake() as conn:
        try:
            work(conn)
        except Error as error:
            work(error)
        finally:
            return work(count)
    pause()
