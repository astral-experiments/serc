def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            x = work_again()
        x = finish()
    return consume(x)
