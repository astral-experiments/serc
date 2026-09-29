def probe(items, out):
    for value in items:
        if 0 <= value < 10:
            out[0] += 1
        elif 10 <= value < 20:
            out[1] += 1
    return out
