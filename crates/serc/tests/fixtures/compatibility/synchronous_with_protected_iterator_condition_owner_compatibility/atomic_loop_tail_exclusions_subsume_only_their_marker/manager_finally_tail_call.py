def f(items, manager):
    with manager:
        try:
            work()
        finally:
            for item in items:
                if check(item):
                    consume(item)
