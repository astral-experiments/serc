def f(x, y):
    for q in y:
        try:
            x()
        except Exception as error:
            pass
        finally:
            pass
