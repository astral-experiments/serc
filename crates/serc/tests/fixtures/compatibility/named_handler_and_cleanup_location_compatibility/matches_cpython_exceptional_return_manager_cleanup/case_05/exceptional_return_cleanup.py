def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        except Error:
            work(count)
        else:
            work(flag)
        finally:
            return work(count)
    pause()
