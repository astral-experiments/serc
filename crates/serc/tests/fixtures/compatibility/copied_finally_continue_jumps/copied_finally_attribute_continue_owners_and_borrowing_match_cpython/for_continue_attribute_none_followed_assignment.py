async def f(items):
    for item in items:
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None:
                close(item)
            item.closed = True
