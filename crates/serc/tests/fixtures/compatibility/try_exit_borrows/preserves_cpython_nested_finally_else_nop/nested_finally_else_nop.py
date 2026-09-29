def f(x, y):
    try:
        action(y)
    finally:
        try:
            action(y)
        except ValueError:
            pass
        else:
            if False:
                action(y)
    return consume(y)
