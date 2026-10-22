def read_value(enabled, manager):
    if enabled:
        try:
            with manager as source:
                value = source.read()
        except OSError:
            return
    else:
        return
    return value.strip()


def index_value(enabled, manager, key):
    if enabled:
        try:
            with manager:
                value = load()
        except OSError:
            return
    else:
        return
    return value[key]
