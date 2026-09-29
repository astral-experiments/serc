def target(manager, work, cleanup, enabled):
    with manager:
        try:
            work()
        finally:
            cleanup()
