def f(x):
    try:
        try:
            consume(x)
        finally:
            pass
        pass
        pass
    except ValueError:
        recover()
