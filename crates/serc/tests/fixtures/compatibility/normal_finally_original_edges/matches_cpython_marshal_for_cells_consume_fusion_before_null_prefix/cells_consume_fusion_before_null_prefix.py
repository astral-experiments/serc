def target():
    first_value = make_value()
    second_value = make_value(make_value())

    def capture():
        return first_value, second_value

    context = make_context()
    observe()
    first, second = make_pair(make_more())
    try:
        assert second == ()
    finally:
        cleanup()
