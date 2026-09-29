def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    return value
            finally:
                consume('cleanup')
            consume('later')
            if retry:
                break
