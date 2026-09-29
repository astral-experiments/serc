def f(manager, item):
    try:
        with manager:
            try:
                work()
            except ValueError as error:
                report(error)
                return
    finally:
        if item is not None:
            item.close()
