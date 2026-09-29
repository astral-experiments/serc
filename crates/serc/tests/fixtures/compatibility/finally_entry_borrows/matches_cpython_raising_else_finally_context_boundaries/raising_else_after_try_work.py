def f(x, cond, other, source):
    try:
        if cond:
            work()
        else:
            raise Error
    except Error:
        raise
    finally:
        consume(x)
    consume(other)
