def probe(selector, outer, left, right):
    if outer and (right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
