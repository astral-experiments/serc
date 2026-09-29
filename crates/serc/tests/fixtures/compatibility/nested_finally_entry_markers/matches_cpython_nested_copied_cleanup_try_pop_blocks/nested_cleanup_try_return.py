def probe(inner, outer, value):
    try:
        try:
            return value
        finally:
            try:
                inner.close()
            except Exception:
                pass
    finally:
        try:
            outer.close()
        except Exception:
            pass
