def target(manager, work, cleanup, enabled):
    with manager:
        try:
            work()
        finally:
            if enabled:
                try:
                    cleanup()
                except ValueError:
                    pass
