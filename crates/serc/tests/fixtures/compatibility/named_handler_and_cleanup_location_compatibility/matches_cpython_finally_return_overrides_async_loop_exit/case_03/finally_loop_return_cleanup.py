async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
                continue
            finally:
                return value
    finish_after()
