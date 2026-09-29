def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            left, right = work()
    work()
