async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
                yield item
            except Error:
                break
            finally:
                return
    finish_after()
