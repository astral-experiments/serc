def f(x):
    try:
        x = x + 1
        pass
    except ValueError:
        recover()
