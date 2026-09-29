def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return factory('effect')
            consume(value)
        consume('later')
        if retry:
            break
