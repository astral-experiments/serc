async def run(manager, source, value):
    async with manager:
        try:
            async for item in source:
                if selected(item):
                    break
                consume(item)
        except Error:
            pass
        consume(value)
        consume(source)
