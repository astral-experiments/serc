def inspect_data(enabled, touch, recover):
    try:
        if enabled:
            global SHARED_VALUE
        touch()
    except ValueError:
        recover()
