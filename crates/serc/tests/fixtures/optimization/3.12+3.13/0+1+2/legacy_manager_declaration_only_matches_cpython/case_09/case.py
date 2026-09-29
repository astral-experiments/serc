def f(manager):
    with manager as value:
        pass
    return value
