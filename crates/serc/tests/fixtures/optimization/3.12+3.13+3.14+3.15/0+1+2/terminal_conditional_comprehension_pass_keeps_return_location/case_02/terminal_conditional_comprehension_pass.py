def f(xs, flag):
    try:
        z = {v for v in xs} if not flag else []; pass
    except ValueError:
        recover()
