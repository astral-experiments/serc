async def run(manager, source, sink):
    async with manager:
        try:
            try:
                async for item in await source():
                    break
            except Error as error:
                if bad(error) or worse(error):
                    raise Wrapped(error) from error
                record(error)
            try:
                await sink(source)
            except Error:
                pass
        except Error:
            recover(source)
    consume(source)
