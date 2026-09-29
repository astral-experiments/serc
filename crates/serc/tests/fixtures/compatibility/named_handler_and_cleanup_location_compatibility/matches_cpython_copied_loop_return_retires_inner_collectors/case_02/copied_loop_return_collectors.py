def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    except Error:
        while flag:
            try:
                break
            finally:
                return count
    pause()
