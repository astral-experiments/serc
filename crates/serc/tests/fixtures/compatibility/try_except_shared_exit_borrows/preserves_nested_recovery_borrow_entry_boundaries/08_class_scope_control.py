class C:
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception:
        consume(source)
        raise
    consume(value)
