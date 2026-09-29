def f(manager):
    with manager:
        global marker
        global spare
    return 7
