def target():
    value = make_value((1, 2, 3))
    captured = value
    extra = None

    def capture():
        return captured, extra

    first, second = make_pair()
    try:
        assert second == ()
    finally:
        cleanup()
