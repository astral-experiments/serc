def f(a, b, result):
    try:
        work()
    except E:
        recover()
    else:
        if a:
            work()
        else:
            if b:
                other()
    after()
