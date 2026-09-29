def f(x, y):
    for q in y:
        try:
            x()
        except:
            pass
        finally:
            pass
