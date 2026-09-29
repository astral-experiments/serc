def outer():
    cell = None
    def f(cm, x, y, value):
        with cm:
            try:
                if x:
                    if y:
                        work()
                    nonlocal cell
                else:
                    other()
            finally:
                cleanup()
            assert value
    return f
