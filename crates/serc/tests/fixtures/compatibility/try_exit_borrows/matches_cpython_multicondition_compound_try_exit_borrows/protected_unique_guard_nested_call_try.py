def probe(value, ready, enabled):
    try:
        if ready:
            try:
                observe(value)
            except Exception:
                pass
    except Exception:
        pass
    consume(value)
