def run(trace, value):
    try:
        if trace:
            observe(value)
        pass
    except Exception:
        recover()
        raise
    consume(value)
    return value
