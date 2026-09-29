def run(enter, trace, value):
    try:
        if enter:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
