def f(manager, flag):
    with manager:
        try:
            work()
        finally:
            if flag:
                cleanup()
            global marker
