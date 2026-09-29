def f(flag, values):
    return (
        any(x for x in values)
        if flag
        else False
    )
