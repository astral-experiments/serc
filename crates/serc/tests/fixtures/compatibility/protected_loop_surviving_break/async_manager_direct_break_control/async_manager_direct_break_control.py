async def run(manager, value):
    async with manager:
        while True:
            result = await receive()
            break
        consume(result, value)
        return result
