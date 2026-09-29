def f(xs, flag):
    count = 0
    with make() as conn:
        while True:
            try:
                work(conn)
            except Error:
                work(count)
            else:
                break
            finally:
                return count
    if count:
        work(count)
    pause()
