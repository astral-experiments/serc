def f(flag, work, recover, finish):
    try:
        if flag:
            return recover()
        else:
            first = work()
            second = work()
    except ValueError:
        return recover()
    else:
        return finish(first, second)
