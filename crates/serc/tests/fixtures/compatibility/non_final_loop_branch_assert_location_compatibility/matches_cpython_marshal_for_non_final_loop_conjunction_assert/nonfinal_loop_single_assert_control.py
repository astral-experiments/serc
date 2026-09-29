def probe(items, outer, first, second, work):
    for item in items:
        if outer(item):
            assert first(item)
        else:
            work(item)
