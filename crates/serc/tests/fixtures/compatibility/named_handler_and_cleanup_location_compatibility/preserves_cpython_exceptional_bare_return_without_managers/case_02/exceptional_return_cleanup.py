def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    except* Error as error:
        work(error)
    finally:
        return
    pause()
