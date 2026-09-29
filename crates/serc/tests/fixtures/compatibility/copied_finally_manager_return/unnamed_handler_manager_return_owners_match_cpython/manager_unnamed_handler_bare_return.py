def f(manager, item):
    try:
        with manager:
            try:
                work()
            except ValueError:
                report(item)
                return
    finally:
        if item is not None:
            item.close()
