def f(manager, value, ready, fallback):
    with manager:
        try:
            return value
        except Error:
            return None
