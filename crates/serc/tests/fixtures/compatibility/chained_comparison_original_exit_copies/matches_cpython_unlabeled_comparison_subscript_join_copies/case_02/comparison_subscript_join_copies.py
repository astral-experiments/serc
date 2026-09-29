def probe(gate, other, a, b, c, values):
    if gate:
        if other:
            index = 1 if a < b < c else 2
        else:
            index = 3
    else:
        index = 4
    return values[index]
