async def run(manager, value):
    async with manager:
        while True:
            item = await receive()
            if item:
                try:
                    result = parse(item)
                    break
                except Error:
                    report(item)
                    continue
            else:
                report(item)
                continue
        consume(result, value)
        return result
