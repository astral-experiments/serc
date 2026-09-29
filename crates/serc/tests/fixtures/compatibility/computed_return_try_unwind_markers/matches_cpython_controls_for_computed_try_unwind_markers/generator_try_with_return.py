def f(manager, value):
    yield value
    with manager:
        try:
            return value
        except Error:
            return None
