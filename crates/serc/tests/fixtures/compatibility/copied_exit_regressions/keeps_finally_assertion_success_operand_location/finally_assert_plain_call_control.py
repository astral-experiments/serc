async def target(operation, record):
    try:
        await operation.run()
    finally:
        record()
        assert operation.done()
    raise AssertionError("unreachable")
