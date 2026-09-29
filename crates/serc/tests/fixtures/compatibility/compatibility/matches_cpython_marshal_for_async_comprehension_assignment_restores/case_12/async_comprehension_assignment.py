async def f(chunks):
    value = [chunk async for chunk in chunks]
    def read():
        return value
    return read
