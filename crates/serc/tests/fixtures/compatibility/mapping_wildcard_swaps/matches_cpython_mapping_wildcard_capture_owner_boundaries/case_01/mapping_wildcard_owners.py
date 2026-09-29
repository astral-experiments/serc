def f(subject):
    try:
        match subject:
            case {'a': x, 'b': y, 'd': _}:
                consume(x, y)
    except Exception:
        recover()
