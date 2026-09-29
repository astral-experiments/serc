def f(manager, value):
    try:
        with manager:
            return value
    except Error:
        raise
