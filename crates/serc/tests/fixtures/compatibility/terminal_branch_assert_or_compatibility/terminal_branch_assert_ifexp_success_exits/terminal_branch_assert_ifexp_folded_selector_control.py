def terminal_true_selector(enabled, value):
    if enabled:
        assert value is not None if True else value is None


def terminal_false_selector(enabled, value):
    if enabled:
        assert value is not None if False else value is None
