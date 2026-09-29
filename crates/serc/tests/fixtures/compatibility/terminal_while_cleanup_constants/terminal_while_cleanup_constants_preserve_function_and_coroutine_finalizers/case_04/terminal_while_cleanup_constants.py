async def probe(manager, nested, factory, managers, slots, items, value, stop, retry, bound):
    while stop:
        with manager:
            try:
                if stop:
                    continue
            finally:
                consume('cleanup')
            consume('later')
            if stop:
                break
