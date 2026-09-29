def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with factory(x for x in ('iterable',)):
            if stop:
                break
            consume('later')
