def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            first()
            work()
    work()
