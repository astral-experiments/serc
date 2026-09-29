def f(flag, active, items):
    if flag is not None:
        assert flag
    if active is not None:
        assert active
        for item in items:
            if item is None:
                continue
            item.consume()
