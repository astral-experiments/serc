async def target(operation, record):
    try:
        await operation.run()
    finally:
        record()
        assert not operation.done()
    raise AssertionError("unreachable")
