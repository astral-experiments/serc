def f(value, values):
    try:
        match value:
            case (t, None, u):
                observe(t, u)
            case _:
                observe(value)
    except Error:
        recover()
