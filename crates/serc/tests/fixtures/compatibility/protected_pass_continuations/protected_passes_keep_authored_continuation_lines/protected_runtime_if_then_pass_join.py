def run(trace, value):
    try:
        if trace:
            observe(value)
        pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
