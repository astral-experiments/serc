def run(load, recover, pending, accept):
    try:
        value = load()
    except RuntimeError:
        value = None
        while pending():
            value = recover()
            if accept(value):
                break
    return value is not None, value
