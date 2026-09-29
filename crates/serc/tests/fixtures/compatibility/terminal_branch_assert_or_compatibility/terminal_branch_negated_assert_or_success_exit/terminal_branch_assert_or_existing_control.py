def terminal_else_or(enabled, left, right):
    if enabled:
        prepare()
    else:
        assert left or right
