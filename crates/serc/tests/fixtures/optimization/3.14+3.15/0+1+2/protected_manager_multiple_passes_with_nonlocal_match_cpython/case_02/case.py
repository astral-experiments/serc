def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                nonlocal marker
                pass
        finally:
            cleanup()
    return inner
