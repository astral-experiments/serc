def f(manager, items, done):
    try:
        for item in items:
            try:
                with manager:
                    return item
            except ValueError:
                pass
    finally:
        if done:
            done()
