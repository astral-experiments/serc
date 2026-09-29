def tuple_boolean_marker(left, right):
    try:
        result = predicate(left) or predicate(right)
    except (ValueError, TypeError):
        result = False
    return result


def single_boolean_marker(left, right):
    try:
        result = predicate(left) or predicate(right)
    except ValueError:
        result = False
    return result


def assignment_handler(flag):
    try:
        if flag:
            value = first
        else:
            prepare()
            value = second
    except AttributeError:
        value = False
    consume(value)


def nonfinal_assignment_branch(flag):
    try:
        if flag:
            value = first
        else:
            prepare()
            value = second
        inspect(value)
    except AttributeError:
        value = False
    consume(value)


def sequential_tries(first, second):
    try:
        value = load(first)
        if first and second:
            if ready(value):
                return value
            else:
                raise ValueError(value)
    except (TypeError, ValueError):
        pass

    try:
        consume(value)
    except RuntimeError:
        pass
    return value
