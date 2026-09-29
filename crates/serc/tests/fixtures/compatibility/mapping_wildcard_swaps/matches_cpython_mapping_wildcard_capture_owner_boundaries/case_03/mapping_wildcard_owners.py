def f(subject):
    global x, y
    match subject:
        case {'a': x, 'b': y, 'd': _}:
            consume(x, y)
