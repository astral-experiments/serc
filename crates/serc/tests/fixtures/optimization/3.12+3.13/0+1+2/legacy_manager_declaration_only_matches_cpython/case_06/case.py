def f(manager):
    with manager as value:
        assert predicate()
        global marker
    return value
