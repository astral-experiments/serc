def f(manager, work):
    try:
        with manager:
            while True:
                work()
    except KeyboardInterrupt:
        return 0
    finally:
        pass
