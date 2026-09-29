def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as first, factory(value) as slots['target']:
            if stop:
                break
            consume('later')
