def f(manager, value, ready, fallback):
    with manager:
        try:
            return build(value)
        except Error:
            return None
