def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            result = work()
        else:
            first()
            result = first()
    return result
