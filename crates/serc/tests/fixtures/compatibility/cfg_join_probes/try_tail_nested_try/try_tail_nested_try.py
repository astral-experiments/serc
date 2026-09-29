def f(flag):
    try:
        if flag:
            try:
                consume()
            except InnerError:
                pass
    except OuterError:
        recover()
    sink(1)
    sink(2)
    return finish()
