def run(retry, trace, value):
    try:
        while True:
            if retry:
                continue
            else:
                if trace:
                    observe(value)
                pass
            consume(value)
            return value
    except Exception:
        recover()
        raise
