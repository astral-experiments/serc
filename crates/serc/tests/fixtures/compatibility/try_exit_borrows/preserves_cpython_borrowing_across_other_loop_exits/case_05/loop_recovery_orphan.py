def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while not True:
            try:
                x = work_again()
            except InnerError:
                x = None
        x = finish()
    return consume(x)
