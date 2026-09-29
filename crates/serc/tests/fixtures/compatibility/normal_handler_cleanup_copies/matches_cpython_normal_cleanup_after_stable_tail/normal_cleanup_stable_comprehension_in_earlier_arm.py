def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = {item: item for item in items}
        else:
            first()
            value = work()
    work()
