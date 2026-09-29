def terminal_ifexp(enabled, condition, value):
    if enabled:
        assert value is not None if condition else value is None
