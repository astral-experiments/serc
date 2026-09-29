
def f(items):
    assert items
    values = prepare(items)
    for item in values:
        first, second = item, item
        assert first
        if first == 1 or second == 2:
            assert second
