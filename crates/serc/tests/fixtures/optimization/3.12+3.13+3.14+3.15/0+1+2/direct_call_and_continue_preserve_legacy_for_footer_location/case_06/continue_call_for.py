def f(flag, active, items):
    if flag is not None:
        consume(flag)
    if active is not None:
        assert active
        for item in items:
            if item is None:
                continue
            consume(item)
