def outer():
    z = None
    def f(xs, flag):
        nonlocal z
        try:
            z = [v for v in xs] if flag else []
            pass
        except ValueError:
            recover()
    return f
