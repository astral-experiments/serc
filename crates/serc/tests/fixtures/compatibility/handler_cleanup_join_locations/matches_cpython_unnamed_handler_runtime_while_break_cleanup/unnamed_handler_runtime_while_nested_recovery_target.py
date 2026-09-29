def run(load, recover, limit):
    attempt = 0
    try:
        value = load()
    except RuntimeError:
        value = None
        while attempt < limit:
            attempt += 1
            try:
                candidate = recover(attempt)
            except ValueError as error:
                report(error)
                continue
            value = candidate
            break
    return finish(value is not None, value)
