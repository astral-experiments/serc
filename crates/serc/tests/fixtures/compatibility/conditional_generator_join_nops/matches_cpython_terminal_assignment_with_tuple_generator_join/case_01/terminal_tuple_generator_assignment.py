result = (
    tuple(f(x) for x in g())
    if cond == value
    else ()
)
