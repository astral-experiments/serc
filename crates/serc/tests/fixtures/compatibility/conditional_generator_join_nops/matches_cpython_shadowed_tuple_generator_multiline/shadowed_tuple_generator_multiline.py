def f(flag, values, tuple):
    return (
        tuple(x for x in values)
        if flag
        else ()
    )
