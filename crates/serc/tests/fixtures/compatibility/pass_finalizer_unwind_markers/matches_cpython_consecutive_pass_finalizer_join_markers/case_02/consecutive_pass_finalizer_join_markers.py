async def collect(value, manager, other, items, flag):
    yield value
    try:
        try:
            try:
                return
            finally:
                async with manager:
                    consume(value)
        finally:
            pass
    finally:
        pass
