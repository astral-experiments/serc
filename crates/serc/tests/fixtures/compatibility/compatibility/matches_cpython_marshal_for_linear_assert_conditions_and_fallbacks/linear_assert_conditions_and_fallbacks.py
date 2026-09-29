def comparisons(value, other, items):
    assert value == other
    assert value < other
    assert value in items
    assert value is other
    assert value is not other
    assert value.method() != other.method()
    assert (
        value.attribute
        == other.attribute
    )

def truth_tests(value, items, predicate):
    assert value
    assert value.attribute
    assert items[0]
    assert predicate(value)

def fallbacks(value, other, predicate):
    assert value is None
    assert value is not None
    assert not value
    assert value and other
    assert value or other
    assert value if predicate() else other
    assert value < other < predicate()
    assert (value, other)
    assert __debug__

def protected(manager, values, predicate):
    for value in values:
        assert predicate(value)
    with manager:
        assert predicate(values)
    try:
        assert predicate(values)
    except Exception:
        assert predicate(values), 'message'

def generator(values, predicate):
    yield values
    assert predicate(values)
