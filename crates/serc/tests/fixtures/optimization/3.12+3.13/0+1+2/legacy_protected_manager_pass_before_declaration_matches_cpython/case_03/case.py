def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with outer:
                with manager:
                    global spare
                    pass
                    global marker
        except Exception:
            cleanup()
        return
    return f
