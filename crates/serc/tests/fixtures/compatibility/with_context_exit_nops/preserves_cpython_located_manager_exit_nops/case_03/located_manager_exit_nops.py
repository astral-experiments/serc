def f(a, b, xs, flag):
    with a:
        try:
            work()
        except ValueError:
            return 1
    done()
