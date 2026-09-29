def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        except* Error as error:
            work(error)
        finally:
            work(count)
            return count
    pause()
