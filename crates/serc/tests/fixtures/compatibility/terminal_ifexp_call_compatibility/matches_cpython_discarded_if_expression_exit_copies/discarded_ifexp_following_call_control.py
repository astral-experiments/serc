def target(outer, choose, left, right):
    if outer:
        left() if choose else right()
        following()
    else:
        fallback()
    finish()
