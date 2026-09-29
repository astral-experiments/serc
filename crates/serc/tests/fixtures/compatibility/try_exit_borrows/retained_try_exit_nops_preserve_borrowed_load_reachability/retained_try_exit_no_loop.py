def f(manager, flag, x):
    with manager:
        try:
            result = check()
            if result:
                return True
            else:
                work()
        except Error:
            recover()
    consume(x)
