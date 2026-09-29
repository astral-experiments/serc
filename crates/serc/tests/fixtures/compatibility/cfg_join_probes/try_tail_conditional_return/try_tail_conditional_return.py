def f(flag):
    try:
        if flag:
            return read()
    except Error:
        recover()
    sink(1)
    sink(2)
    return finish()
