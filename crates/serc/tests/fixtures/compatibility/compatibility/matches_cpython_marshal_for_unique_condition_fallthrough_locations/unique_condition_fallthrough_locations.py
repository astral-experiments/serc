def terminal_try(left, right):
    try:
        assert left and right
    except TypeError:
        pass


def terminal_branch(flag, left, right):
    if flag:
        assert left and right
    else:
        consume()


def protected(value):
    with manager(invalid="ignore"):
        result = transform(value)
        if result.real != 0 or result.imag != 0:
            raise AssertionError(format_result(result.real, result.imag))


def nested_join(flag, left, right):
    if flag:
        if not (left and right):
            raise ValueError
    elif alternate:
        consume()


def nested_loop(items, left, right):
    for item in items:
        while left or right:
            left, right = update(item)
