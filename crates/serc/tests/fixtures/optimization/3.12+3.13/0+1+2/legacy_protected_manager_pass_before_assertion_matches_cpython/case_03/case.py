def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner:
                pass
                pass
                nonlocal outer_marker
                assert True
        finally:
            cleanup()
        return
    return f
