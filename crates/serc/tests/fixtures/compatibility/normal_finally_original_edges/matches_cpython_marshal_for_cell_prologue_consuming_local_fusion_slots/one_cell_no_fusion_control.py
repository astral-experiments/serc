def target():
    value = make_value((1, 2, 3))
    captured = value

    def capture():
        return captured

    first = make_pair()
    try:
        assert first == ()
    finally:
        cleanup()
