def target(enabled, choose, left, right):
    if enabled:
        result = left if choose else right
    else:
        fallback()
