def f(xs, flag):
    count = 0
    with make() as conn:
        try:
            work(conn)
        finally:
            return
    pause()
