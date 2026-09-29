def f(manager, value, ready, fallback):
    with manager:
        return value if ready else fallback
