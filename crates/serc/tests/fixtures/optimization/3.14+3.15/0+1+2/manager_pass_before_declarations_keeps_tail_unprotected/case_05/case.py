def f(manager, other):
    with manager:
        global marker
        pass
        global spare_marker
    return 42
