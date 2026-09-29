async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            if stop:
                return 'answer'
            consume('later')
            if retry:
                break
