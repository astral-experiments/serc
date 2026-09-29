
def preceding_filter(values, replacement):
    return [replacement if value is None else value for value in values if value]
