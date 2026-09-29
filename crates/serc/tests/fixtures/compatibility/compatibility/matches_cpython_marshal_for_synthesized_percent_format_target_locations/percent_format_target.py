def terminal_false_target(flag, value):
    if flag:
        return value
    return "tail=%s" % value


def else_target(flag, value):
    if flag:
        result = "left=%s" % value
    else:
        result = "right=%s" % value
    return result


def final_elif_target(first, second, value):
    if first:
        return first
    elif second:
        return second
    return "tail=%s" % value


def entry_block(value):
    return "entry=%s" % value


def true_fallthrough(flag, value):
    if flag:
        return "body=%s" % value
    return value


def formatted_first(flag, value):
    if flag:
        return flag
    return "%s=tail" % value


def filtered_generator(items):
    return "".join(" %s=%s" % (key, value) for key, value in items if value)
