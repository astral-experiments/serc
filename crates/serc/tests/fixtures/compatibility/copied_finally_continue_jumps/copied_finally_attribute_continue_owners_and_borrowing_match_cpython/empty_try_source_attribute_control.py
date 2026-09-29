async def f(items):
    for item in items:
        try:
            pass
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
            touch(item)
