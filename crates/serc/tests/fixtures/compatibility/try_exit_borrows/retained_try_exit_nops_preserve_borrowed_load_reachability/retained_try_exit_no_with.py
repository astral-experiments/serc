def f(manager, flag, x):
    try:
        result = check()
        if result:
            return True
        else:
            work()
    except Error:
        recover()
    consume(x)
