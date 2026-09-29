def probe(value, ready, enabled):
    try:
        if ready and enabled:
            try:
                observe(value)
            except ValueError as error:
                if not expected(error):
                    raise
                recover(error)
    except Exception:
        return None
    consume(value)
    finish(value)
    return value
