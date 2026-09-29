def assert_only_control(value):
    try:
        pass
    except:
        if True:
            assert False
    value = None
