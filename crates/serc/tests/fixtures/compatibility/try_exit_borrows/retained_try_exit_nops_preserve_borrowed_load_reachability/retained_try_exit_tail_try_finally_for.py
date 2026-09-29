def f(manager, items, x):
    with manager:
        try:
            for item in items:
                work(item)
        finally:
            cleanup()
    return x
