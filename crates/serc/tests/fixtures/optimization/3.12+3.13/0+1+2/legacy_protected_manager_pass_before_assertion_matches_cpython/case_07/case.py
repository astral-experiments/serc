def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as value: pass; assert predicate()
        finally:
            cleanup()
        return
    return f
