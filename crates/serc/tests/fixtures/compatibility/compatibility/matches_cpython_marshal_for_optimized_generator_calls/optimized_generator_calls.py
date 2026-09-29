module_any = any(value for value in values)
module_all = all(value for value in values)
module_tuple = tuple(value for value in values)
any(value for value in values)
all(value for value in values)
tuple(value for value in values)
after = None


def optimized(values):
    return (
        any(value for value in values),
        all(value for value in values),
        tuple(value for value in values),
    )


def rebound(values, any):
    return any(value for value in values)


def terminal_discard(values):
    all(value for value in values)
