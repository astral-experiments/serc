def f(a, b, flag):
    with (
        a,
        b,
        a,
        b,
    ):
        try:
            return flag
        except Error:
            raise
    done()
