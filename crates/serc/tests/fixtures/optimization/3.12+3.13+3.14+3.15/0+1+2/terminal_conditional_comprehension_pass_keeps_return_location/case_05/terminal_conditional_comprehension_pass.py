def f(xs, flag):
    try:
        try:
            pass
        except TypeError:
            recover_inner()
        z = [v for v in xs if v] if flag else []
        pass
    except ValueError:
        recover()
