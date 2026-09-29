def f(manager, other):
    with other:
        with manager as value:
            pass
            pass
            global marker
    return value
