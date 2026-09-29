def f(items, outer, inner, flag):
    for item in items:
        try:
            return []
        finally:
            if flag:
                with inner:
                    cleanup()
