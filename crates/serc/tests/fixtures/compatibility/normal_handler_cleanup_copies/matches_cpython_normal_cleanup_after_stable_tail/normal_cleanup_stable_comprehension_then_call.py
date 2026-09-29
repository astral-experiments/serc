def probe(work, first, flag, items):
    try:
        work()
    except ValueError:
        first()
        if flag:
            first()
            value = work()
        else:
            value = {item: item for item in items}
            first(value)
    work()
