def target():
    a = make_value((1, 2))
    b = make_value()
    c = make_value()
    d = make_value()
    e = make_value()
    f = make_value()

    def capture():
        return a, b, c, d, e, f

    first, second = make_pair()
    third, fourth = make_pair()
    fifth, sixth = make_pair()
    observe()
    observe()
    observe()
    observe()
    try:
        assert a.wait(timeout=1)
    finally:
        cleanup()
