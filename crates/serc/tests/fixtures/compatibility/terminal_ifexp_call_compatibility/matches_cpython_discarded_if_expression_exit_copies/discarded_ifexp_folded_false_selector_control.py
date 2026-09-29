def target(outer, left, right):
    if outer:
        left() if False else right()
    else:
        fallback()
    finish()
