def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            if b:
                work()
        else:
            other()
    after()
