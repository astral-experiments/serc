def f(value, emit):
    handle = None
    path = None
    try:
        handle = acquire()
        path = prepare(value)
        work(handle, path)
    finally:
        if handle is not None:
            try:
                release(handle)
            except Error:
                pass
        if path is not None and path.exists():
            try:
                path.remove()
            except Error:
                pass
    emit(value)
    return value
