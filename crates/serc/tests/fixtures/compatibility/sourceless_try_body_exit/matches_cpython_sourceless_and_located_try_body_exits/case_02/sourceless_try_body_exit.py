def f(a, b, result):
    try:
        if a:
            if b:
                work()
        else:
            other()
    except E:
        recover()
    after()
