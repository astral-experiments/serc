def target(outer, inner, choose, left, right):
    if outer:
        if inner:
            fallback()
        else:
            result = left if choose else right
    else:
        finish()
