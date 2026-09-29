def load_pair(first, second, item):
    consume(first, second, message=("Expected %s" % item))

def store_pair(items):
    return ["pair %s/%s" % (first, second) for first, second in items]

def reversed_store_pair(first, second):
    left, right = convert(first), convert(second)
    return "pair %s/%s" % (left, right)
