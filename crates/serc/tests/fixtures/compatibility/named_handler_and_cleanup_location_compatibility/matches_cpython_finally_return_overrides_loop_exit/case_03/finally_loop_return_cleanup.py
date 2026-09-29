def f(xs, flag):
    count = 0
    with make(), make() as conn:
        for item in xs:
            try:
                continue
            finally:
                return count
    if count:
        work(count)
    pause()
