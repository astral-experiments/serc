def f(enabled, value, recover, tail):
    if enabled:
        try:
            assert value.ready
        except Exception as error:
            recover(error)
    tail()
