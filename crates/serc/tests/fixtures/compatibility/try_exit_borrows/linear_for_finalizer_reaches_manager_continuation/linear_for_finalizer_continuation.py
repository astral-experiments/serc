def f(manager, items, value, work, observe):
    with manager:
        try:
            for item in items:
                work(item)
        finally:
            for item in items:
                item.cancel()
    observe(value)
    observe(value)
    return value
