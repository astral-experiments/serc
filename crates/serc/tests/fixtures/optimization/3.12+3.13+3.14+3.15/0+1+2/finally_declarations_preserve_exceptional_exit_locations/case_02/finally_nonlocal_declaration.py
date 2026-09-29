def outer():
    marker = None
    def f(manager, flag):
        with manager:
            try:
                work()
            finally:
                if flag:
                    cleanup()
                nonlocal marker
    return f
