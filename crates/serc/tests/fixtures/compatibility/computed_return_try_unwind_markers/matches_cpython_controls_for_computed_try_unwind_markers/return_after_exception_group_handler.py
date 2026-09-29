def f(manager, value):
    with manager:
        try:
            work()
        except* Error:
            record()
        return value
