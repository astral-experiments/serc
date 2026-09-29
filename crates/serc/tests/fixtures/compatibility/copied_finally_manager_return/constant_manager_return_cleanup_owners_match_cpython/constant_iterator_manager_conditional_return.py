async def example(manager, items, value):
    try:
        with manager:
            for item in items:
                try:
                    if should_return(item):
                        tick()
                        return
                except ValueError:
                    pass
    except RuntimeError:
        raise
    finally:
        if value is not None:
            finish(value)
        after()
