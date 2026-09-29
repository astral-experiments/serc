def probe(value, ready, enabled, manager):
    try:
        if ready:
            with manager:
                return value
    except Exception:
        pass
    consume(value)
