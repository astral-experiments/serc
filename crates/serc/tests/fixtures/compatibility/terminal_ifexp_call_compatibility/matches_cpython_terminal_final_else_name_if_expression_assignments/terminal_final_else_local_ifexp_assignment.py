def target(enabled, preferred, fallback):
    if enabled:
        result = preferred
    else:
        result = fallback if fallback else None
