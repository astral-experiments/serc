def f(manager, items, value, work, observe):
    with manager:
        try:
            work(value)
        finally:
            for item in items:
                try:
                    item.cancel()
                except Exception:
                    pass
    observe(value)
    observe(value)
    return value
