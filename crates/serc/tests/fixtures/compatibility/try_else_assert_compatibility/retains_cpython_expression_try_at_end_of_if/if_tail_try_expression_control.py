def f(enabled, work, recover, tail):
    if enabled:
        try:
            work()
        except Exception as error:
            recover(error)
    tail()
