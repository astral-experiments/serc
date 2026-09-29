async def stream(items, emit):
    try:
        for item in items:
            try:
                while True:
                    try:
                        yield item
                        return
                    except ValueError:
                        raise
                    finally:
                        emit(item)
            except TypeError:
                pass
    except RuntimeError:
        raise
    finally:
        emit(None)
