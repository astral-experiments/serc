def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        with manager as value:
            pass
            assert predicate()
        return
    return f
