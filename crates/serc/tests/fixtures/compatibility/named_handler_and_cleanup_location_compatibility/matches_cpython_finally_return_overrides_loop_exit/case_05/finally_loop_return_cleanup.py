def f(xs, flag):
    count = 0
    conn = flag
    for item in xs:
        try:
            work(conn)
        except Error as error:
            break
        finally:
            return count
    if count:
        work(count)
    pause()
