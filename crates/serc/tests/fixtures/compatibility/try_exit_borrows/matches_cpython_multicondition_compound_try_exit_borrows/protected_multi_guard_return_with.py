def probe(value, ready, enabled, manager):
    try:
        if ready and enabled:
            with manager:
                return value
    except Exception:
        pass
    consume(value)
