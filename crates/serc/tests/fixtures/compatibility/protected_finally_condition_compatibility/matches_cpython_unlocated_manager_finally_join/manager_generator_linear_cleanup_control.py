def managed_value(manager, cleanup):
    with manager():
        try:
            yield
        finally:
            cleanup()
