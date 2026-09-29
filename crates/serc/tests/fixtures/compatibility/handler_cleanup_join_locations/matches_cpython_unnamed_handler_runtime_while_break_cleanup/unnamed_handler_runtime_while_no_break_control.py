def run(load, recover, pending):
    try:
        value = load()
    except RuntimeError:
        value = None
        while pending():
            value = recover()
    return value is not None, value
