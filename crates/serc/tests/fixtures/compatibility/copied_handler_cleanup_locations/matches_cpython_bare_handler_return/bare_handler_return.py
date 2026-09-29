def f(value, action):
    try:
        action()
    except Exception:
        if value:
            consume(value)
    return value
