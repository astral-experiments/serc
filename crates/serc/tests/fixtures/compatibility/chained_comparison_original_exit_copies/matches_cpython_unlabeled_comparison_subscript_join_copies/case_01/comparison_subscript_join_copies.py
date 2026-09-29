def probe(gate, other, a, b, c, values):
    if gate:
        if a < b < c:
            if other:
                index = 1
            else:
                index = 2
        else:
            index = 3
    else:
        index = 4
    return values[index]
