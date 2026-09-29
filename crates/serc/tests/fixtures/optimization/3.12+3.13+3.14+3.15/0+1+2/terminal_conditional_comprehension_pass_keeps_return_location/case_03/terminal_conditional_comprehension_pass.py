def f(xs, flag):
    try:
        z = {v: v for v in xs} if check(flag) else []
        pass
    except ValueError:
        recover()
