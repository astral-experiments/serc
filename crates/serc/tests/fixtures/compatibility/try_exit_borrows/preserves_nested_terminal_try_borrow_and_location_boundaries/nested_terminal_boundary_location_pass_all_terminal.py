def f(flag, value):
    error = None
    try:
        work()
    except Outer:
        pass
    else:
        if flag:
            try:
                return parse()
            except Inner as exc:
                return str(exc)
    if value:
        emit(value)
    return error
