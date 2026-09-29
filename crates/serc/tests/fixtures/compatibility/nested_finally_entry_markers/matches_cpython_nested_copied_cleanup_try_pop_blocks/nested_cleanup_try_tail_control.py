def probe(inner, outer, observe, value):
    try:
        try:
            return value
        finally:
            try:
                inner.close()
            except Exception:
                pass
            observe()
    finally:
        try:
            outer.close()
        except Exception:
            pass
