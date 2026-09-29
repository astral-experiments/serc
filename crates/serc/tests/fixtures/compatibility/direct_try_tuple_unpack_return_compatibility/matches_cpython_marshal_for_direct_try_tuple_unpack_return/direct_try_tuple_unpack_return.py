def tuple_unpack(platform):
    try:
        first, _, _ = platform.first_version()
        if first:
            return first
        _, second, _, _ = platform.second_version()
        if second:
            return second
    except OSError:
        pass
    return ""


def simple_assignment_control(platform):
    try:
        first = platform.first_version()
        if first:
            return first
        _, second, _, _ = platform.second_version()
        if second:
            return second
    except OSError:
        pass
    return ""


def named_handler_control(platform):
    try:
        _, value, _ = platform.first_version()
        if value:
            return value
    except OSError as error:
        raise LookupError from error
    return ""
