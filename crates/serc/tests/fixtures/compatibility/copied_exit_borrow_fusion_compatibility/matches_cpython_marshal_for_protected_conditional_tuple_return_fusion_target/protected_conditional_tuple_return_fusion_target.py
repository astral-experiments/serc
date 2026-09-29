def f(view, path, extra):
    try:
        return view, path if exists(path) else None, extra
    except Error as error:
        record(error)
        return None, None, None
