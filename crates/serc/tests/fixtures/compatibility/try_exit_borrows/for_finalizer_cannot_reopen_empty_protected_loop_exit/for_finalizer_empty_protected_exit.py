def f(manager, items, value, observe):
    with manager:
        try:
            while True:
                break
        finally:
            for item in items:
                item.cancel()
    observe(value)
    observe(value)
    return value
