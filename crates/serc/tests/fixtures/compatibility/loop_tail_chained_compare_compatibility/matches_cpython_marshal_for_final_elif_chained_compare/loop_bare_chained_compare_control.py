def probe(items, out):
    for value in items:
        if 10 <= value < 20:
            out[1] += 1
    return out
