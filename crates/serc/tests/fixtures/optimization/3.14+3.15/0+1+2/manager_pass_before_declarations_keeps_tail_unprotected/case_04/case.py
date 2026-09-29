def outer():
    captured = 1
    def f(manager, other):
        with manager as value:
            pass
            pass
            nonlocal captured
        return value
    return f
