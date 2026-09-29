def after_handler(items):
    for item in items:
        if item:
            try:
                value = item()
            except Error:
                pass
            break
    return None, None, None


def terminal_branch_before_break(items, first, second):
    for item in items:
        try:
            value = item()
            if first and second:
                return value
        except Error:
            pass
        break
    return None, None, None


def direct_control(items):
    for item in items:
        if item:
            value = item()
            break
    return None, None, None


def named_tuple_control(items):
    first = None
    second = None
    for item in items:
        if item:
            first = item
            break
    return first, second
