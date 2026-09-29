def f(value):
    def capture():
        return value
    ready = value
    try:
        result = consume(ready) if ready else []
    except Exception:
        result = []
    return capture, result
