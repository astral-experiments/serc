def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager as value:
                pass
                global marker
        finally:
            cleanup()
        return
    return f
