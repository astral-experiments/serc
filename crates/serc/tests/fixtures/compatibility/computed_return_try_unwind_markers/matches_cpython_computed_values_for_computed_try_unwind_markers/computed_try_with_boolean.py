def f(manager, value, ready, fallback):
    with manager:
        try:
            return value and fallback
        except Error:
            return None
