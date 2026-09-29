def f(xs, flag):
    count = 0
    with make() as conn:
        while True:
            try:
                break
            finally:
                return count
    if count:
        work(count)
    pause()
