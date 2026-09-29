def run(retry, trace, value):
    while True:
        if retry:
            continue
        else:
            if trace:
                observe(value)
            pass
        consume(value)
        return value
