def f(work, manager, flag):
    with manager:
        try:
            work()
        except Exception as error:
            if not flag:
                assert False, error
        work()
