def f(enabled, left, right, work, recover, tail):
    if enabled:
        work()
    else:
        try:
            assert left and right
        except Exception as error:
            recover(error)
    tail()
