def f(a, b, flag, other, value, items, manager, cause):
    try:
        work(value)
        return value
    except Error:
        if b:
            report(value)
            value = None
        else:
            report(other)
            raise Error(value)
    finally:
        if flag:
            consume(value, other)
    consume(value, other)
