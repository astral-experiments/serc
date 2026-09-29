def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with outer:
                with inner:
                    assert predicate()
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
