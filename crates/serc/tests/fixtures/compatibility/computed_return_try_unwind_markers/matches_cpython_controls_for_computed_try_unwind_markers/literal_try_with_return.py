def f(manager):
    with manager:
        try:
            return None
        except Error:
            return 1
