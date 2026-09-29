def outer(manager):
    marker = 1
    def inner():
        with manager as value:
            nonlocal marker
        return value
    return inner
