def f(x):
    try:
        del x
        pass
    except ValueError:
        recover()
