def protected_terminal_or(enabled, left, right):
    try:
        if enabled:
            assert left or right
    except Exception:
        recover()
