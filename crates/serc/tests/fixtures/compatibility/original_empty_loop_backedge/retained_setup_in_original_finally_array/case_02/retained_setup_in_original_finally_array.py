def f(flag):
    with manager():
        try:
            work()
        except BaseException:
            raise
        finally:
            try:
                if flag:
                    cleanup()
            except OSError:
                recover()
