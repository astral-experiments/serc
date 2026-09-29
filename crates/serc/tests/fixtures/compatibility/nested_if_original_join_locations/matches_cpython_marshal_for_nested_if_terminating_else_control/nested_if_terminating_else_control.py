def f(outer, inner):
    if outer:
        if inner:
            first()
    else:
        return
    finish()
