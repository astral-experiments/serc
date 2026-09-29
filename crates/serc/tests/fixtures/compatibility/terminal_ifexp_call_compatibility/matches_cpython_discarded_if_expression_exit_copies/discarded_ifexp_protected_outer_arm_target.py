def target(outer, choose, left, right):
    try:
        if outer:
            left() if choose else right()
        elif choose:
            fallback()
        finish()
    except Exception:
        recover()
    return None
