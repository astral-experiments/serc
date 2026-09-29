def f(manager, work, cleanup):
    try:
        raise LookupError
        with manager:
            while True:
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
