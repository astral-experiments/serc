def collect(mode, pending, take, stop):
    result = []
    if mode:
        while pending():
            value = take()
            result.append(value)
            if stop(value):
                break
    else:
        result.append(None)
    return result
