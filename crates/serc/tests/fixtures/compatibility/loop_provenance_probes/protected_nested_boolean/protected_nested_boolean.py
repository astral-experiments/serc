def target(items, left, middle, right, consume):
    for item in items:
        try:
            if left(item) or (middle(item) and right(item)):
                consume(item)
        except Exception:
            pass


def single_false_edge(items, left, right, consume):
    for item in items:
        try:
            if left(item) or right(item):
                consume(item)
        except Exception:
            pass
