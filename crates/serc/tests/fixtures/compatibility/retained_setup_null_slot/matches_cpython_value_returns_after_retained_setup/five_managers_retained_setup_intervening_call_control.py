def f(a, b, c, d, e, extra, flag, value, fallback, audit, finish):
    if flag:
        audit()
    with a, b, c, d, e:
        try:
            audit()
            result = value or fallback()
            return result
        finally:
            finish()
