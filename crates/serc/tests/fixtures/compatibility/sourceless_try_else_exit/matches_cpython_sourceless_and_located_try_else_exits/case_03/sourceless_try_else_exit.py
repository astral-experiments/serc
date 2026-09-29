def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            work()
        elif b:
            other()
    after()
