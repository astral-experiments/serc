def f(x, cond, other, source):
    try:
        work()
    except Error:
        try:
            if cond:
                work()
            else:
                raise Error
        except Error:
            raise
        finally:
            consume(x)
