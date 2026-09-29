def probe(items, outer, first, second, work):
    for item in items:
        if outer(item):
            assert first(item) and second(item)
        else:
            work(item)
