def probe(work, first, flag):
    result = None
    try:
        work()
    except ValueError:
        first()
        if flag:
            try:
                result = work()
            except ValueError:
                first()
                result = first()
        else:
            first()
            result = first()
    first()
    return result
