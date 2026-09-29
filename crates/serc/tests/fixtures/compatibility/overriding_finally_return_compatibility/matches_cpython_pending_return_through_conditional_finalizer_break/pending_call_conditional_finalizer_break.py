def f(items, factory, value, stop):
    for item in items:
        try:
            return factory()
        finally:
            if stop:
                break
            return value
    return None
