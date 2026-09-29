def target():
    value = make_value((1, 2, 3))
    captured = value

    def capture():
        return captured

    first, second = make_pair()
    try:
        assert second == ()
    finally:
        cleanup()
