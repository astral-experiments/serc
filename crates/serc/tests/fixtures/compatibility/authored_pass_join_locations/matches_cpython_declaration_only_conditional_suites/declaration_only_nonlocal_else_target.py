def outer():
    value = 0
    def inspect_data(enabled, touch):
        if enabled:
            touch()
        else:
            nonlocal value
        return value
    return inspect_data
