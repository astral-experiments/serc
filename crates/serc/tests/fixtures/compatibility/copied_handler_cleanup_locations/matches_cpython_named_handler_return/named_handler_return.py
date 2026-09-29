def f(value, action):
    try:
        action()
    except Exception as exc:
        if value:
            consume(exc)
    return value
