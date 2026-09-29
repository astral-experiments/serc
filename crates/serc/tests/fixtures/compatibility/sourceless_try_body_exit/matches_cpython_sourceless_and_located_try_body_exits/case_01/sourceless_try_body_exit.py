def f(a, b, result):
    try:
        if a:
            work()
        else:
            if b:
                other()
    except E:
        recover()
    after()
