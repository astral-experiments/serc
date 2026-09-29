async def probe(operation, ready):
    try:
        await operation()
    except ValueError as original:
        try:
            code = await operation()
            assert code == 0
        except TypeError as recovery:
            print(recovery)
        else:
            print(code)
        if ready(original):
            raise RuntimeError() from original
        raise
