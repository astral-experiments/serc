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
        if False:
            consume(reader)
    finally:
        pass
    consume(reader)
