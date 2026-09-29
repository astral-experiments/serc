def f(x, cond, other, source):
    try:
        try:
            if cond:
                work()
            else:
                raise Error
        except Error:
            raise
        finally:
            consume(x)
    except BaseException:
        raise
