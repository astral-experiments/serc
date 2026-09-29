def probe(work, a, b, flag):
    try:
        work()
    except ValueError:
        value = 1
        if flag:
            value = a()
            b(value)
    work()
