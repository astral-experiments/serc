def probe(work, a, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
    work()
