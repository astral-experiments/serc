def f(manager, value, ready, fallback):
    with manager:
        try:
            return value.result
        except Error:
            return None
