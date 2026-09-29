def f(manager, flag):
    with manager:
        try:
            yield
        except ValueError:
            flag = False
            raise
        finally:
            if flag:
                close()
                wait()
