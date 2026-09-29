def probe(value, ready, enabled):
    try:
        if ready and enabled:
            try:
                observe(value)
            except Exception:
                pass
    except Exception:
        pass
    consume(value)
