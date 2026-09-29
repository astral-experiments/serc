def f(xs, flag):
    global z
    try:
        z = [(w := v) for v in xs] if flag else []
        pass
    except ValueError:
        recover()
