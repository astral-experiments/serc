async def example(manager, value):
    try:
        with manager:
            try:
                work()
            except ValueError as error:
                await cancel()
                yield Event(text='failure')
                return
    finally:
        if value is not None and not value.done():
            finish(value)
        after()
