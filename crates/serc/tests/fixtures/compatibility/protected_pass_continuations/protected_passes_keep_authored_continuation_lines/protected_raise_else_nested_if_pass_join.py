def run(fail, trace, value):
    try:
        if fail:
            raise ValueError
        else:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
    except Exception:
        recover()
        raise
