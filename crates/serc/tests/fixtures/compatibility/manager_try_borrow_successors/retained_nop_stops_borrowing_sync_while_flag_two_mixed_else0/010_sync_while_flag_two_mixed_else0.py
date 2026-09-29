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
                pass
            try:
                consume(other)
            except InnerError:
                consume(other)
    return value, other
