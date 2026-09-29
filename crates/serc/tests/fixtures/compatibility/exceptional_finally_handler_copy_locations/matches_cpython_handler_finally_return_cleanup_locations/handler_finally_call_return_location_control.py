def f(load, manager, cleanup):
    try:
        return load()
    except ValueError:
        try:
            with manager():
                return load()
        finally:
            cleanup()
