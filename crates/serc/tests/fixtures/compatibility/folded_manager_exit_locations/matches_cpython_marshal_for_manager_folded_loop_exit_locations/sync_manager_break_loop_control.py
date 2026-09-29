def f(manager, ready, work, cleanup):
    try:
        with manager:
            while True:
                if ready():
                    break
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        cleanup.close()
