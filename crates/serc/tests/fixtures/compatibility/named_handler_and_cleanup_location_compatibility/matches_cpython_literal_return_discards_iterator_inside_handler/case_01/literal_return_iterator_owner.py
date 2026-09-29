def f(xs, flag):
    count = 0
    try:
        work(flag)
    except Error:
        for item in xs:
            try:
                break
            finally:
                return None
    pause()
