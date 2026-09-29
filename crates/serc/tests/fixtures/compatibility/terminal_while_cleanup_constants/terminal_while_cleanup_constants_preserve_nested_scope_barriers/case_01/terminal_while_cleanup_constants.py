def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with factory(lambda x='default': x):
            if stop:
                break
            consume('later')
