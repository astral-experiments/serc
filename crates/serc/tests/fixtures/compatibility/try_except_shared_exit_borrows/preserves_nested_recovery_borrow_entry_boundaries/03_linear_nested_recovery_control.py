def f(source, consume):
    try:
        try:
            value = source.first()
        except ValueError:
            value = source.fallback()
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
