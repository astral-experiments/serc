def f(a, b):
    try:
        if a and b:
            return read()
    except Error:
        recover()
    sink(1)
    sink(2)
    return finish()
