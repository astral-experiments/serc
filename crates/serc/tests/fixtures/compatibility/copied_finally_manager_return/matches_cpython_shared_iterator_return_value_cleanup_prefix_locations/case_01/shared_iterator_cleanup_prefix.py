def f(items, outer, inner, flag):
    with outer:
        for item in items:
            try:
                return []
            finally:
                if flag:
                    with inner:
                        cleanup()
