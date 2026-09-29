def probe(manager, value, stop, retry):
    while stop:
        with manager:
            try:
                consume(value)
            except Error as caught:
                if retry:
                    continue
            else:
                consume('else-first')
            if stop:
                break
