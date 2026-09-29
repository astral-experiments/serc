def outer(flag):
    def f():
        try:
            work()
            work()
            work()
            work()
        finally:
            if flag is None:
                cleanup()
    return f
