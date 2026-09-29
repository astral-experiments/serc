def f(manager, value, ready, fallback):
    with manager:
        try:
            return (value, fallback)
        except Error:
            return None
