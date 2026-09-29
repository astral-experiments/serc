async def collect(value, manager, items, flag):
    yield value
    try:
        try:
            try:
                prepare(value)
            except Error as error:
                observe(error)
                await work(value)
                return
        finally:
            pass
    finally:
        with manager:
            pass
