def terminal_multiline_selector(enabled, value):
    if enabled:
        assert (
            value is not None
            if True
            else value is None
        )
