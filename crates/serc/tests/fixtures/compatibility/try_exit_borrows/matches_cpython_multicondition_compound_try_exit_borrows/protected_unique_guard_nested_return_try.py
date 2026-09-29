def probe(value, ready, enabled):
    try:
        if ready:
            try:
                result = decode(value)
                if valid(result):
                    return result
            except Exception:
                pass
    except Exception:
        pass
    consume(value)
