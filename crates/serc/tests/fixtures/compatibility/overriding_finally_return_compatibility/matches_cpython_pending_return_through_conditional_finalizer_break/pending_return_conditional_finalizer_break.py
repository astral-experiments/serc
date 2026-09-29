def f(items, value, stop):
    for item in items:
        try:
            value += item
            if item < 0:
                break
            return value
        finally:
            if stop:
                break
            return value
    return None
