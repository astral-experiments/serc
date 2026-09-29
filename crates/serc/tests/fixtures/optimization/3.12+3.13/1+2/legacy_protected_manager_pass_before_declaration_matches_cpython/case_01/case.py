def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                assert predicate()
        finally:
            cleanup()
        return
    return f
