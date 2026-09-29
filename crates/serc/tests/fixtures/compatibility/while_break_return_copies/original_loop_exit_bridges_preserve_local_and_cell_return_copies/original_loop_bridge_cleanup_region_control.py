def collect(mode, pending, take, stop, release):
    result = []
    if mode:
        try:
            while pending():
                value = take()
                result.append(value)
                if stop(value):
                    break
        finally:
            release()
    else:
        result.append(None)
    return result
