def f(flag, values):
    return (
        all(x for x in values)
        if flag
        else False
    )
