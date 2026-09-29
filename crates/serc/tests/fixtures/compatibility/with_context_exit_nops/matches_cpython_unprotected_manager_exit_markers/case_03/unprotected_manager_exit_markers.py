def f(a, b, flag):
    with (
        a,
        b,
        a,
    ):
        try:
            raise Error()
        except Error:
            return flag
    done()
