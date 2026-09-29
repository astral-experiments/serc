def f(a, b, xs, flag):
    with a:
        with b:
            try:
                work()
            except ValueError:
                return 1
    done()
