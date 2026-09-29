def f(xs, flag):
    try:
        z = [v for v in xs] if flag else []
        pass
    except ValueError:
        recover()
