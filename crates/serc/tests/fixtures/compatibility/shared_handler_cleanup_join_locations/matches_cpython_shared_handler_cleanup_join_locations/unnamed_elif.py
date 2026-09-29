def f(work, a, b, after):
    try:
        work()
    except ValueError:
        if a:
            raise
        elif b:
            value = work()
    after()
