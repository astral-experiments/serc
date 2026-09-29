async def f(items):
    while True:
        item = await items.get()
        retained = False
        try:
            work(item)
        except Error as error:
            if should_retry(item):
                continue
        finally:
            if item.resource is not None and not retained:
                close(item)
            touch(item)
