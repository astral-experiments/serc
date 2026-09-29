def outer():
    value = 0
    def inspect_data(enabled):
        if enabled:
            nonlocal value
        return value
    return inspect_data
