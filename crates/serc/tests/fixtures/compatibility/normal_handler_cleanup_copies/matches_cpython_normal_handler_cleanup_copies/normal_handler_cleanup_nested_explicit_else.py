def probe(work, a, b, flag, inner):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = 2
            if inner:
                a()
            else:
                b()
    work()
