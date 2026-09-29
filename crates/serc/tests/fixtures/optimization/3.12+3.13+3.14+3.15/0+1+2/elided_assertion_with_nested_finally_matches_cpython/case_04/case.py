def outer(flag):
    declared = None
    def f():
        try:
            operation()
            nonlocal declared
            assert flag
        except ValueError:
            raise
        finally:
            try:
                cleanup()
            except TypeError:
                pass
    return f
