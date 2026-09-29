def f(x, cond, other, source):
    try:
        if cond:
            work()
        else:
            raise Error
    except Error as error:
        if other:
            raise
    finally:
        consume(x)
