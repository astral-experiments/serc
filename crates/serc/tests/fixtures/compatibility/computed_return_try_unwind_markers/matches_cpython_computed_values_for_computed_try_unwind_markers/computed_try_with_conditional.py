def f(manager, value, ready, fallback):
    with manager:
        try:
            return value if ready else fallback
        except Error:
            return None
