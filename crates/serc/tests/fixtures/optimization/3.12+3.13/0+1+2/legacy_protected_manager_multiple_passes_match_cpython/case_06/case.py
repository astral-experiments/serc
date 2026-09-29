def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
