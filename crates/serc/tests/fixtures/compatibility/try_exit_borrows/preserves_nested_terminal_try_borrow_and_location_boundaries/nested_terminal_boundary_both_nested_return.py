def f(flag, value):
    error = None
    try:
        work()
    except Outer:
        error = "outer"
    else:
        if flag:
            try:
                return parse()
            except Inner as exc:
                error = str(exc)
        else:
            try:
                return fail()
            except Other:
                error = "other"
    if value:
        emit(value)
    return error
