def f(subject, manager):
    with manager:
        match subject:
            case {'a': x, 'b': y, 'd': _}:
                consume(x, y)
