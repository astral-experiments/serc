def f(a, b, result):
    try:
        if a:
            work()
        else:
            other()
    except E:
        recover()
    after()
