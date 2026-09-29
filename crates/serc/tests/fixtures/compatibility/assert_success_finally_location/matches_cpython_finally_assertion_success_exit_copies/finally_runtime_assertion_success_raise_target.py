def check(value, cleanup, wanted):
    try:
        if value:
            return value
    finally:
        item = cleanup()
        assert item == wanted
    raise AssertionError
