def f(flag, values):
    return (
        ()
        if flag
        else tuple(x for x in values)
    )
