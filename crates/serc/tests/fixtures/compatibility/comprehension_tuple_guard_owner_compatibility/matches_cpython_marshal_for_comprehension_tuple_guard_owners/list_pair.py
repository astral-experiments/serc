
def list_pair(outer, values, left, right):
    return [((left, right), tuple(x for x in values))[1] for y in outer]
