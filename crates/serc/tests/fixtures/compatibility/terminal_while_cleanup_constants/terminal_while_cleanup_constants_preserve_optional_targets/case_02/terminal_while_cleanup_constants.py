def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager as slots[:bound]:
            if stop:
                break
            consume('later')
