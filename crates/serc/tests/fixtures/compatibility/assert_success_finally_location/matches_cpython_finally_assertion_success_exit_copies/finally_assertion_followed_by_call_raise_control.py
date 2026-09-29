def check(value, cleanup, wanted, finish):
    try:
        if value:
            return value
    finally:
        item = cleanup()
        assert item == wanted
        finish()
    raise AssertionError
