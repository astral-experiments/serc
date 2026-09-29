def f(x, cond, other, source):
    try:
        if False:
            work()
        else:
            raise Error
    except Error:
        raise
    finally:
        consume(x)
