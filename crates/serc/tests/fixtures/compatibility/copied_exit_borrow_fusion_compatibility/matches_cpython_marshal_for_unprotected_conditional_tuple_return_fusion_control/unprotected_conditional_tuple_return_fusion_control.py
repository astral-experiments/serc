def f(view, path, extra):
    return view, path if exists(path) else None, extra
