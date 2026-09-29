def f(value, recover, tail):
    try:
        assert value.ready
    except Exception as error:
        recover(error)
    tail()
