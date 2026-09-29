def collect(stream):
    values = []
    try:
        try:
            while stream.ready():
                values.append(stream.take())
        except Exception:
            values = []
    except Exception:
        values = []
    return values
