async def read_chunks(manager, failed):
    if failed:
        yield 'unavailable'
        return
    with manager() as stream:
        try:
            value = stream.read()
            assert value is not None
        except Exception as error:
            yield str(error)
    yield value
