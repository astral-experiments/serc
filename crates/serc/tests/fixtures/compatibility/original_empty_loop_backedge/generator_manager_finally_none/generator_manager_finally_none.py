def f(manager, pending):
    with manager:
        try:
            yield
        finally:
            if pending is not None:
                pending.close()
