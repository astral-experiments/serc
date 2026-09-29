def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        with inner:
            pass
            assert predicate()
        return
    return f
