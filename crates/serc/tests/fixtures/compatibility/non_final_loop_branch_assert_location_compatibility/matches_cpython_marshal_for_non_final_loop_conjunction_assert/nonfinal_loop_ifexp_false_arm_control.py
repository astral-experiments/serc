def probe(items, outer, first, selector, work):
    for item in items:
        if outer(item):
            assert (first(item) if selector(item) else False)
        else:
            work(item)
