def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            left, right = None, None
        else:
            first()
            value = work()
    work()
