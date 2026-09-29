def probe(manager, factory, value, stop, retry):
    while stop:
        try:
            with manager:
                if stop:
                    return value
                consume(value)
        finally:
            consume('cleanup')
        consume('later')
        if retry:
            break
