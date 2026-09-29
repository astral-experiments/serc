def f(a, b, flag):
    try:
        with (
            a,
            b,
            a,
        ):
            try:
                return flag
            except Error:
                raise
    except Error:
        raise
