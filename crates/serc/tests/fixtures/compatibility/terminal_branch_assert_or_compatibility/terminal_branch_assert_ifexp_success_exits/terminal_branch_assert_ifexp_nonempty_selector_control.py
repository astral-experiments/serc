def terminal_nonempty_selector(enabled, value):
    if enabled:
        prepare()
        assert value is not None if True else value is None
