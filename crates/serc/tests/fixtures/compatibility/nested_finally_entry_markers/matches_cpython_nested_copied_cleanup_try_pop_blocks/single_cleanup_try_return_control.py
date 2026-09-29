def probe(inner, value):
    try:
        return value
    finally:
        try:
            inner.close()
        except Exception:
            pass
