def probe(selector, outer, left, right):
    if outer and (left or right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
