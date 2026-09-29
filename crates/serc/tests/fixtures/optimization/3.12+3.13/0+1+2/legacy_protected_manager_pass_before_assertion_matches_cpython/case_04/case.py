def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as obj.value:
                pass
                assert False
        except Exception:
            cleanup()
        return
    return f
