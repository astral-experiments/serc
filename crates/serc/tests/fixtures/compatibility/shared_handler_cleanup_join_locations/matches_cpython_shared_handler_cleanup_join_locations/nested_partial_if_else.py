def f(work, flag, cached, after):
    try:
        return work()
    except NameError:
        if flag:
            value = work()
            if cached:
                return value
            else:
                value = work()
                work(value)
    after()
