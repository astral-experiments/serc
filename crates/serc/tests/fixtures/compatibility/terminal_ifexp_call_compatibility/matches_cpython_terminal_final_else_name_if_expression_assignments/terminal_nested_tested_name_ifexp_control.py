def target(outer, inner, choose, left, right):
    if outer:
        fallback()
    else:
        if inner:
            result = left if choose else right
        else:
            finish()
