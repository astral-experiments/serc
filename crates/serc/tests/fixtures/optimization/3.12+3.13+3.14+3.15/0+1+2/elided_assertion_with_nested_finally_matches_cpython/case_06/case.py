def outer():
    x = None
    def f():
        try:
            operation()
            nonlocal x
        except ValueError:
            raise
        finally:
            try:
                cleanup()
            except TypeError:
                pass
    return f
