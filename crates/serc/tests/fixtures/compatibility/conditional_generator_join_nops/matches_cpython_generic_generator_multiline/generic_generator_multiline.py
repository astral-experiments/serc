def f(flag, fn, values):
    return (
        fn(x for x in values)
        if flag
        else ()
    )
