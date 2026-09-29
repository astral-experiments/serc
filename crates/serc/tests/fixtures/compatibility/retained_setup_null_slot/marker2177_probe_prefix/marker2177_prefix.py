def f(manager, flag):
    if flag is None:
        yield
        return
    flag = True
    with manager:
        prepare()
        try:
            yield
        except ValueError:
            report()
            flag = False
            raise
        finally:
            if flag:
                close()
                wait()
