async def run(reader, other):
    try:
        while True:
            done = await poll()
            if other is not None and other in done:
                await consume(other.result())
                other = None
            if reader in done:
                item = reader.result()
                if item is None:
                    break
                async for output in handle(item):
                    yield output
                reader = make_reader()
        if other is not None:
            await consume(await other)
        await persist(reader)
    finally:
        if not reader.done():
            reader.cancel()
            await gather(reader)
        if other is not None and not other.done():
            other.cancel()
            await gather(other)
