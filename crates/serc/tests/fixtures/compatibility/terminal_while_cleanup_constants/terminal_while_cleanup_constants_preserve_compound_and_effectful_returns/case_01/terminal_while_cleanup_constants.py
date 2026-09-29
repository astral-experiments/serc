def probe(manager, factory, value, stop, retry):
    while stop:
        with manager:
            if stop:
                return 'an' + 'swer'
            consume(value)
        consume('later')
        if retry:
            break
