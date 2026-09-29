async def collect(value, manager, outer):
    try:
        try:
            try:
                return work(value)
            finally:
                42
        finally:
            with manager:
                pass
    except Error:
        recover()
