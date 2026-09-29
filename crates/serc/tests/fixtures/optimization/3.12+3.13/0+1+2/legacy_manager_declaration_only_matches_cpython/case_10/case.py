def f(manager):
    with manager as value: global marker
    return value
