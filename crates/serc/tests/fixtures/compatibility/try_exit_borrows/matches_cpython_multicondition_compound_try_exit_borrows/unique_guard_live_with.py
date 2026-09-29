def probe(value, ready, enabled, manager):
    try:
        if ready:
            with manager:
                observe(value)
    except Exception:
        recover(value)
    consume(value)
