def target(outer, choose, left, right):
    if outer:
        left() if choose else right()
    else:
        fallback()
    finish()
