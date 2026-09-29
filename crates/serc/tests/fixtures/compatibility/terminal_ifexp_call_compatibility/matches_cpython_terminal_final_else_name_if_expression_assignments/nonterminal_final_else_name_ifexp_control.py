def target(enabled, choose, left, right):
    if enabled:
        fallback()
    else:
        result = left if choose else right
    finish()
