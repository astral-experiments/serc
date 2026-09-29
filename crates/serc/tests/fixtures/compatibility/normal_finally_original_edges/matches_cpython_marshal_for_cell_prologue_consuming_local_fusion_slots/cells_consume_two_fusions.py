def target():
    value = make_value((1, 2))
    captured = value
    extra = None

    def capture():
        return captured, extra

    first, second = make_pair()
    third, fourth = make_pair()
    try:
        assert fourth == ()
    finally:
        cleanup()
