def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return -7
            consume(value)
        consume('later')
        if retry:
            break
