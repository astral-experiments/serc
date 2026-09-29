def f(flag, values):
    return (
        list(x for x in values)
        if flag
        else []
    )
