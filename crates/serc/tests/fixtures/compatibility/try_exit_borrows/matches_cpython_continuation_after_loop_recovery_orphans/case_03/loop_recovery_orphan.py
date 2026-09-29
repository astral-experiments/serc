def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            try:
                x = work_again()
            except InnerError as inner_error:
                x = inner_error
        x = finish()
    return consume(x)
