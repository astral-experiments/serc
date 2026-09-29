def f(flag):
    try:
        value = begin()
        if flag:
            try:
                consume(value)
            except InnerError:
                pass
    except OuterError:
        try:
            recover()
        except InnerError:
            pass
        value = 0
    sink(value)
    sink(2)
    return finish()
