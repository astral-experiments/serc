def f(x):
    try:
        x = x + 1
        pass; pass
    except ValueError:
        recover()
