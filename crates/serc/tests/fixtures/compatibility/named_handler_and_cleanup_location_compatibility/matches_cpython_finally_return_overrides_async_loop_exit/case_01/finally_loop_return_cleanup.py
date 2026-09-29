async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
            except Error:
                break
            finally:
                return await finish(value)
    finish_after()
