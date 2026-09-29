def f(manager):
    with manager as value:
        global marker
        assert predicate()
    return value
