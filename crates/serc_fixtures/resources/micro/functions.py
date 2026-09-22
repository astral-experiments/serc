def decorator(function):
    return function


@decorator
def parameters(a, b=1, /, c=2, *args, d, e=3, **kwargs):
    return a, b, c, args, d, e, kwargs


def closures(seed):
    total = seed

    def add(value):
        nonlocal total
        total += value
        return total

    return add


identity = lambda value: value
combine = lambda left, right=1: left + right
