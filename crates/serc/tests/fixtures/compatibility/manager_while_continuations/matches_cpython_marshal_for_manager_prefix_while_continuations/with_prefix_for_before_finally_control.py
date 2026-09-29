def f(manager, values, work, cleanup):
    with manager:
        for value in values:
            work(value)
        try:
            work(None)
        finally:
            cleanup()
