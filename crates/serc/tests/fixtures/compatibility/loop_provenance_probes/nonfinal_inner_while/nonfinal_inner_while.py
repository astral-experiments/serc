def target(items, first, second, consume):
    for item in items:
        if item:
            while first(item) and second(item):
                consume(item)
        else:
            consume(None)


def single_false_edge(items, first, consume):
    for item in items:
        if item:
            while first(item):
                consume(item)
        else:
            consume(None)
