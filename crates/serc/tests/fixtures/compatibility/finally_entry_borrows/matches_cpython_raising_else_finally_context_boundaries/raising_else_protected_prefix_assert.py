def f(x, cond, other, source):
    try:
        assert True
        if cond:
            work()
        else:
            raise Error
    except Error:
        raise
    finally:
        consume(x)
