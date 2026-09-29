def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        finally:
            return count
    pause()
