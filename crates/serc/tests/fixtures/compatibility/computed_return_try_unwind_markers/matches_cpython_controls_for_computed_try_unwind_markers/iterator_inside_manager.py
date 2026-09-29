def f(manager, values):
    with manager:
        for value in values:
            try:
                return value
            except Error:
                raise
