def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
