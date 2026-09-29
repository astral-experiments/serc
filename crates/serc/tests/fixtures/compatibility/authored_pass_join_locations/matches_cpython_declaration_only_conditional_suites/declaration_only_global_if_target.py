def inspect_data(enabled, debug, values):
    if enabled:
        if debug:
            global PROCESSED_VALUES
        if set(values) == {1}:
            return values
    return None
