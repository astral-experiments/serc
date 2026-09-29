def f(a, b, items):
    assert a
    if check(a):
        assert b
    if a < b < items:
        assert a
        for item in items:
            assert item
