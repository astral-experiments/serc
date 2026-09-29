def f(groups):
    try:
        for group in groups:
            if not group or not any(check(item) for item in group):
                continue
            yield group
    finally:
        cleanup()
