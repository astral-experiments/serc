def f(index, bound, values):
    if 0 <= index < bound:
        return values[index]
    else:
        raise IndexError('x')
