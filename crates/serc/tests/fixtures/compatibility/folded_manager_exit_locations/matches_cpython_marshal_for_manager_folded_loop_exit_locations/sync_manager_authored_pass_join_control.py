def f(manager, work, cleanup):
    try:
        with manager:
            while True:
                work()
        pass
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
