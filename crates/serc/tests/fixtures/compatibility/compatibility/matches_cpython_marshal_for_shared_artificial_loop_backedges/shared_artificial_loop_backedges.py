def try_else(values, output):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            if callback(value):
                output.append(value)
    return output


def try_finally(values):
    for value in values:
        try:
            consume()
        finally:
            if enabled():
                cleanup()
