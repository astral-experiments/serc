def values(items):
    yield 0
    for item in items:
        yield item
    yield 1


def delegated(items):
    yield from values(items)
    yield from (item * 2 for item in items)


generator_lambda = lambda items: (yield from items)
