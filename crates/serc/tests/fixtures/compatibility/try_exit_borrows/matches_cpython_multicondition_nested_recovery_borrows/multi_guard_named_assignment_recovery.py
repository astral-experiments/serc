def probe(value, ready, enabled):
    try:
        if ready and enabled:
            try:
                result = decode(value)
                value = result
            except ValueError as error:
                recover(error)
    except Exception:
        return None
    consume(value)
    finish(value)
    return value
