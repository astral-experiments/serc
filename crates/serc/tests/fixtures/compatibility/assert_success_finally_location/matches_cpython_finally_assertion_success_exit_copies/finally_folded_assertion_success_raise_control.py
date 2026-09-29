def check(value, cleanup):
    try:
        if value:
            return value
    finally:
        cleanup()
        assert True
    raise AssertionError
