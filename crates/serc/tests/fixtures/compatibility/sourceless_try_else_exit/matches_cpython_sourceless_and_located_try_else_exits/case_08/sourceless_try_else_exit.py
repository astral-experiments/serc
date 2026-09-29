def f(a, b, c, result, items=()):
    try:
        work(result)
    except E:
        recover(result)
    else:
        if a and b:
            return result
    after(result)
