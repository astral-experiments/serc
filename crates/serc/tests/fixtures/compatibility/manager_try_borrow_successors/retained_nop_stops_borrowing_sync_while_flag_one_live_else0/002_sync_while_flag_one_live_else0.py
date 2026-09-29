def f(value, other, manager, flag, items, consume, poll):
    with manager:
        try:
            while flag:
                consume(value)
                flag = poll()
        except Error:
            try:
                consume(other)
            except InnerError:
                consume(value)
    return value, other
