def test(xs, y, z):
    yield 0
    for x in xs:
        if x:
            if x < y < z:
                yield x
        else:
            assert z
