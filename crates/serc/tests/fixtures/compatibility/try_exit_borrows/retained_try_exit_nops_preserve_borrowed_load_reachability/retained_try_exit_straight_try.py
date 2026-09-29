def f(manager, flag, x):
    with manager:
        try:
            work()
        except Error:
            recover()
    consume(x)
