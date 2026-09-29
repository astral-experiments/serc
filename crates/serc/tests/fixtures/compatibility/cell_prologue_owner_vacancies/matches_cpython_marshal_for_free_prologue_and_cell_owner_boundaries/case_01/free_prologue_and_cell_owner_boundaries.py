def f(flag, a, b, c, d):
    def capture():
        return flag
    try:
        result = work((0,1,2))
    finally:
        if flag is None:
            cleanup()
