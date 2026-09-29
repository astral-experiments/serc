def f(xs, flag):
    try:
        try:
            work()
        finally:
            pass
        z = [v for v in xs if v] if flag else []
        pass
    except ValueError:
        recover()
