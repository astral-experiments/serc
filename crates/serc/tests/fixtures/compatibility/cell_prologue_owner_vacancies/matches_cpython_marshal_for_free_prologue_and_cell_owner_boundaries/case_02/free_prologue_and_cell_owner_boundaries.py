def f(flag):
    def capture():
        return flag
    if SENTINEL:
        prepare()
    try:
        work()
        work()
        work()
        work()
    finally:
        if flag is None:
            cleanup()
