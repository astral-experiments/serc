def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d, e:
        try:
            result = value and fallback()
            return result
        finally:
            finish()
