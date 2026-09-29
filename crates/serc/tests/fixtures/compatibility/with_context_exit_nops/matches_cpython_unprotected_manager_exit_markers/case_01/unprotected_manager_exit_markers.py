def f(a, b, flag):
    with (
        a,
        b,
        a,
    ):
        try:
            return flag
        except Error:
            raise
    done()
