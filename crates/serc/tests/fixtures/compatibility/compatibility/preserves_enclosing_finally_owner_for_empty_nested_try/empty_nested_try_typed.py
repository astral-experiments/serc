def f(y):
    try:
        try:
            pass
        except ValueError:
            pass
    finally:
        y()
