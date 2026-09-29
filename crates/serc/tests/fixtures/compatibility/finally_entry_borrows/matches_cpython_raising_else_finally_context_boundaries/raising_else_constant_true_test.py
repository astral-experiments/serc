def f(x, cond, other, source):
    try:
        if True:
            work()
        else:
            raise Error
    except Error:
        raise
    finally:
        consume(x)
