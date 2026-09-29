def f(flag, source, consume):
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
        consume(value)
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
