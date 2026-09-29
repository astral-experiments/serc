async def collect(value, manager, other, items, flag):
    try:
        try:
            try:
                return value
            finally:
                async with manager:
                    consume(value)
        finally:
            pass
    finally:
        pass
