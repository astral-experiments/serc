def repair(value, mismatch, enabled, replace, cleanup):
    try:
        if mismatch:
            changed = None
            if enabled:
                changed = replace()
            if changed is not None:
                value = changed
            else:
                raise ValueError()
    except Exception:
        try:
            cleanup()
        except Exception:
            pass
        raise
    return value
