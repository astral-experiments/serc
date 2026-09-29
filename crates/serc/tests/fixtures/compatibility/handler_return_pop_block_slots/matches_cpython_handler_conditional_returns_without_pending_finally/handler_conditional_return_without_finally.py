def f(value, flag):
    try:
        work()
    except Error:
        return value if flag else convert(value)
