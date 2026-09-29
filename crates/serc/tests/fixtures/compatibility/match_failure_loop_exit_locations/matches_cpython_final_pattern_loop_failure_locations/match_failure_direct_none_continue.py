def f(items):
    for item in items:
        match item:
            case Entry(value=value):
                consume(value)
            case None:
                continue
    return None
